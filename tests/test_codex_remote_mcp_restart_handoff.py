from __future__ import annotations

from datetime import UTC, datetime, timedelta
from pathlib import Path
from typing import final, override

import pytest

from codex_remote_mcp_bridge_config import RemoteMcpBridgeConfig
from codex_remote_mcp_bridge import RemoteMcpBridge
from codex_remote_mcp_restart_handoff import (
    RESUME_ENV_NAME,
    claim_restart_handoff,
    write_restart_handoff,
)
from codex_remote_mcp_restart_models import (
    HandoffProtector,
    RestartHandoffError,
    RestartProject,
)
from simdorei_mcp_common.messages import ProjectUpsert
from tests.test_codex_remote_mcp_bridge import FakeConnector, FakeSocket


@final
class _XorProtector(HandoffProtector):
    @override
    def protect(self, payload: bytes) -> bytes:
        return bytes(value ^ 0xA5 for value in payload)

    @override
    def unprotect(self, payload: bytes) -> bytes:
        return bytes(value ^ 0xA5 for value in payload)


def _config() -> RemoteMcpBridgeConfig:
    return RemoteMcpBridgeConfig(
        bridge_url="wss://example.test/v12/bridge",
        device_id="device-a",
        device_token="device-token-must-not-be-stored",
    )


def _restart_project(root: Path, expires_at: datetime) -> RestartProject:
    return RestartProject(
        project=ProjectUpsert(
            project_scope="project-scope-must-not-be-plaintext",
            binding_id="binding-id-must-not-be-plaintext",
            thread_id="thread-a",
            project_name="project-a",
            expires_at=expires_at,
        ),
        root=root,
    )


def test_encrypted_restart_handoff_is_single_use_and_preserves_exact_binding(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    now = datetime.now(UTC)
    handoff_path = tmp_path / "handoff.json"
    project_root = tmp_path / "project-root-must-not-be-plaintext"
    project_root.mkdir()
    project = _restart_project(project_root, now + timedelta(minutes=10))
    protected = _XorProtector()
    secured: list[Path] = []

    assert write_restart_handoff(
        (project,),
        _config(),
        path=handoff_path,
        protector=protected,
        now=lambda: now,
        secure_directory=secured.append,
    )
    raw = handoff_path.read_bytes()
    assert project.project.project_scope.encode() not in raw
    assert project.project.binding_id.encode() not in raw
    assert str(project_root).encode() not in raw
    assert _config().device_token.encode() not in raw
    assert secured == [tmp_path]

    monkeypatch.setenv(RESUME_ENV_NAME, "1")
    restored = claim_restart_handoff(
        _config(),
        path=handoff_path,
        protector=protected,
        now=lambda: now + timedelta(seconds=1),
    )

    assert restored == (project,)
    assert not handoff_path.exists()
    assert (
        claim_restart_handoff(
            _config(),
            path=handoff_path,
            protector=protected,
            now=lambda: now + timedelta(seconds=1),
        )
        == ()
    )


def test_normal_start_does_not_claim_restart_handoff(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    now = datetime.now(UTC)
    handoff_path = tmp_path / "handoff.json"
    project_root = tmp_path / "project"
    project_root.mkdir()
    protected = _XorProtector()
    assert write_restart_handoff(
        (_restart_project(project_root, now + timedelta(minutes=10)),),
        _config(),
        path=handoff_path,
        protector=protected,
        now=lambda: now,
        secure_directory=lambda _: None,
    )
    monkeypatch.delenv(RESUME_ENV_NAME, raising=False)

    assert (
        claim_restart_handoff(
            _config(),
            path=handoff_path,
            protector=protected,
            now=lambda: now,
        )
        == ()
    )
    assert handoff_path.is_file()


def test_expired_restart_handoff_is_consumed_without_restoring_project(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    now = datetime.now(UTC)
    handoff_path = tmp_path / "handoff.json"
    project_root = tmp_path / "project"
    project_root.mkdir()
    protected = _XorProtector()
    assert write_restart_handoff(
        (_restart_project(project_root, now + timedelta(minutes=10)),),
        _config(),
        path=handoff_path,
        protector=protected,
        now=lambda: now,
        secure_directory=lambda _: None,
    )
    monkeypatch.setenv(RESUME_ENV_NAME, "1")

    with pytest.raises(RestartHandoffError, match="expired"):
        _ = claim_restart_handoff(
            _config(),
            path=handoff_path,
            protector=protected,
            now=lambda: now + timedelta(minutes=3),
        )

    assert not handoff_path.exists()


def test_replacement_bridge_replays_exact_project_binding(tmp_path: Path) -> None:
    first_socket = FakeSocket()
    first = RemoteMcpBridge(
        _config(),
        connector=FakeConnector(first_socket),
        log=lambda _: None,
    )
    try:
        _ = first.register_project(
            "thread-a",
            "project-scope-a",
            tmp_path,
        )
        projects = first.prepare_restart_projects()
    finally:
        first.close()

    second_socket = FakeSocket()
    second = RemoteMcpBridge(
        _config(),
        connector=FakeConnector(second_socket),
        log=lambda _: None,
    )
    try:
        ticket = second.restore_project(projects[0])
        replayed = next(
            ProjectUpsert.model_validate_json(message)
            for message in second_socket.sent
            if '"type":"project_upsert"' in message
        )
    finally:
        second.close()

    assert replayed == projects[0].project
    assert ticket.project_scope == projects[0].project.project_scope
    assert ticket.expires_at == projects[0].project.expires_at
