from __future__ import annotations

from pathlib import Path
from typing import final

import pytest

import codex_remote_mcp_binding as binding
from codex_remote_mcp_bridge import LogFunc
from codex_remote_mcp_bridge_config import ProjectTicket, RemoteMcpBridgeConfig
from codex_remote_mcp_bridge_connection import RemoteMcpBridgeError
from codex_remote_mcp_restart_models import RestartProject


@final
class _FakeBridge:  # MUTABLE_OK: deterministic retry-state fake.
    def __init__(self, *, close_fails: bool = False) -> None:
        self.close_fails = close_fails
        self.close_calls = 0
        self.connect_device_calls = 0

    def connect_device(self) -> None:
        self.connect_device_calls += 1

    def close(self) -> None:
        self.close_calls += 1
        if self.close_fails:
            raise RemoteMcpBridgeError("owned cleanup is still pending")

    def register_project(
        self,
        thread_id: str,
        project_scope: str,
        root: Path,
    ) -> ProjectTicket:
        _ = thread_id, project_scope, root
        raise AssertionError("not used by lifecycle tests")

    def prepare_restart_projects(
        self,
        *,
        timeout_seconds: float = 10.0,
    ) -> tuple[RestartProject, ...]:
        _ = timeout_seconds
        return ()

    def cancel_restart_preparation(self) -> None:
        return None

    def restore_project(self, restart_project: RestartProject) -> ProjectTicket:
        _ = restart_project
        raise AssertionError("not used by lifecycle tests")


def _config(device_id: str) -> RemoteMcpBridgeConfig:
    return RemoteMcpBridgeConfig(
        bridge_url="wss://example.test/bridge",
        device_id=device_id,
        device_token="test-token",
    )


def _bridge(fake: _FakeBridge) -> binding.ManagedRemoteMcpBridge:
    return fake


def test_connect_remote_mcp_device_returns_configured_pc_and_folder(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    fake = _FakeBridge()
    config = _config("device-a")
    monkeypatch.setattr(binding, "load_remote_mcp_config", lambda: config)
    monkeypatch.setattr(binding, "_get_bridge", lambda actual, log: _bridge(fake))

    ticket = binding.connect_remote_mcp_device(tmp_path, lambda _: None)

    assert ticket is not None
    assert ticket.device_id == "device-a"
    assert ticket.working_directory == tmp_path.resolve()
    assert fake.connect_device_calls == 1


def test_close_retains_bridge_until_cleanup_retry_succeeds(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    fake = _FakeBridge(close_fails=True)
    monkeypatch.setattr(binding, "_bridge", _bridge(fake))
    monkeypatch.setattr(binding, "_bridge_config", _config("device-a"))

    with pytest.raises(RemoteMcpBridgeError, match="cleanup is still pending"):
        binding.close_remote_mcp_bridge()

    assert binding._bridge is fake
    assert binding._bridge_config == _config("device-a")
    fake.close_fails = False
    binding.close_remote_mcp_bridge()
    assert binding._bridge is None
    assert binding._bridge_config is None
    assert fake.close_calls == 2


def test_config_replacement_waits_for_previous_cleanup(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    previous = _FakeBridge(close_fails=True)
    replacement = _FakeBridge()
    created: list[RemoteMcpBridgeConfig] = []
    monkeypatch.setattr(binding, "_bridge", _bridge(previous))
    monkeypatch.setattr(binding, "_bridge_config", _config("device-a"))

    def create(
        config: RemoteMcpBridgeConfig, *, log: LogFunc
    ) -> binding.ManagedRemoteMcpBridge:
        _ = log
        created.append(config)
        return _bridge(replacement)

    monkeypatch.setattr(binding, "RemoteMcpBridge", create)

    with pytest.raises(RemoteMcpBridgeError, match="cleanup is still pending"):
        _ = binding._get_bridge(_config("device-b"), lambda _: None)

    assert binding._bridge is previous
    assert created == []
    previous.close_fails = False
    result = binding._get_bridge(_config("device-b"), lambda _: None)
    assert result is replacement
    assert binding._bridge is replacement
    assert binding._bridge_config == _config("device-b")
    assert previous.close_calls == 2


def test_config_replacement_does_not_retain_a_closed_bridge_when_start_fails(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    previous = _FakeBridge()
    monkeypatch.setattr(binding, "_bridge", _bridge(previous))
    monkeypatch.setattr(binding, "_bridge_config", _config("device-a"))

    def fail_create(
        config: RemoteMcpBridgeConfig, *, log: LogFunc
    ) -> binding.ManagedRemoteMcpBridge:
        _ = config, log
        raise RemoteMcpBridgeError("replacement could not start")

    monkeypatch.setattr(binding, "RemoteMcpBridge", fail_create)

    with pytest.raises(RemoteMcpBridgeError, match="could not start"):
        _ = binding._get_bridge(_config("device-b"), lambda _: None)

    assert binding._bridge is None
    assert binding._bridge_config is None
    assert previous.close_calls == 1
