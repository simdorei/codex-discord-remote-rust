from __future__ import annotations

import threading
from pathlib import Path
from typing import Protocol

from codex_remote_mcp_bridge import LogFunc, RemoteMcpBridge
from codex_remote_mcp_bridge_config import (
    DeviceTicket,
    ProjectTicket,
    RemoteMcpBridgeConfig,
    load_remote_mcp_config,
)
from codex_remote_mcp_restart_handoff import (
    claim_restart_handoff,
    write_restart_handoff,
)
from codex_remote_mcp_bridge_connection import RemoteMcpBridgeError
from codex_remote_mcp_restart_models import RestartHandoffError, RestartProject

_BRIDGE_LOCK = threading.Lock()


class ManagedRemoteMcpBridge(Protocol):
    def connect_device(self) -> None: ...

    def register_project(
        self,
        thread_id: str,
        project_scope: str,
        root: Path,
    ) -> ProjectTicket: ...

    def close(self) -> None: ...

    def prepare_restart_projects(
        self,
        *,
        timeout_seconds: float = 10.0,
    ) -> tuple[RestartProject, ...]: ...

    def cancel_restart_preparation(self) -> None: ...

    def restore_project(self, restart_project: RestartProject) -> ProjectTicket: ...


_bridge: ManagedRemoteMcpBridge | None = None
_bridge_config: RemoteMcpBridgeConfig | None = None


def connect_remote_mcp_device(
    root: Path,
    log: LogFunc,
) -> DeviceTicket | None:
    config = load_remote_mcp_config()
    if config is None:
        return None
    bridge = _get_bridge(config, log)
    bridge.connect_device()
    return DeviceTicket(
        device_id=config.device_id,
        working_directory=root.resolve(),
    )


def register_remote_mcp_project(
    thread_id: str,
    project_scope: str,
    root: Path,
    log: LogFunc,
) -> ProjectTicket | None:
    config = load_remote_mcp_config()
    if config is None:
        return None
    bridge = _get_bridge(config, log)
    return bridge.register_project(thread_id, project_scope, root)


def close_remote_mcp_bridge() -> None:
    global _bridge, _bridge_config
    with _BRIDGE_LOCK:
        bridge = _bridge
        if bridge is None:
            return
        bridge.close()
        _bridge = None
        _bridge_config = None


def prepare_remote_mcp_restart_handoff(log: LogFunc) -> bool:
    with _BRIDGE_LOCK:
        bridge = _bridge
        config = _bridge_config
    if bridge is None or config is None:
        return False
    projects = bridge.prepare_restart_projects()
    try:
        prepared = write_restart_handoff(projects, config)
    except (OSError, RestartHandoffError, RemoteMcpBridgeError):
        bridge.cancel_restart_preparation()
        raise
    if prepared:
        log(f"remote_mcp_restart_handoff_prepared projects={len(projects)}")
    else:
        bridge.cancel_restart_preparation()
    return prepared


def restore_remote_mcp_restart_handoff(log: LogFunc) -> bool:
    config = load_remote_mcp_config()
    if config is None:
        return False
    projects = claim_restart_handoff(config)
    if not projects:
        return False
    bridge = _get_bridge(config, log)
    try:
        for project in projects:
            _ = bridge.restore_project(project)
    except (OSError, RestartHandoffError, RemoteMcpBridgeError):
        close_remote_mcp_bridge()
        raise
    log(f"remote_mcp_restart_handoff_restored projects={len(projects)}")
    return True


def _get_bridge(config: RemoteMcpBridgeConfig, log: LogFunc) -> ManagedRemoteMcpBridge:
    global _bridge, _bridge_config
    with _BRIDGE_LOCK:
        if _bridge is not None and _bridge_config == config:
            return _bridge
        if _bridge is not None:
            _bridge.close()
            _bridge = None
            _bridge_config = None
        replacement = RemoteMcpBridge(config, log=log)
        _bridge = replacement
        _bridge_config = config
        return replacement
