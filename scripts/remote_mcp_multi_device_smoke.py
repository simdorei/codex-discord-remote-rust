from __future__ import annotations

import json
import os
import sys
import time
import urllib.request
from pathlib import Path
from uuid import uuid4

REPO_ROOT = Path(__file__).resolve().parents[1]
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

from codex_remote_mcp_bridge import RemoteMcpBridge
from codex_remote_mcp_bridge_config import RemoteMcpBridgeConfig


def main() -> int:
    health_url = _required("QA_REMOTE_MCP_HEALTH_URL")
    bridge_url = _required("QA_REMOTE_MCP_BRIDGE_URL")
    project_root = Path(os.environ.get("QA_REMOTE_MCP_ROOT", REPO_ROOT)).resolve()
    bridge_a = _bridge("A", bridge_url)
    bridge_b = _bridge("B", bridge_url)
    replacement_a: RemoteMcpBridge | None = None
    try:
        scope_a = f"multi-device-a-{uuid4().hex}"
        scope_b = f"multi-device-b-{uuid4().hex}"
        _ = bridge_a.register_project("multi-device-thread-a", scope_a, project_root)
        _ = bridge_b.register_project("multi-device-thread-b", scope_b, project_root)
        _wait_for_connected_devices(health_url, 2)

        restart_projects = bridge_a.prepare_restart_projects()
        if len(restart_projects) != 1:
            raise RuntimeError("Device A did not preserve one restart project.")
        bridge_a.close()
        _wait_for_connected_devices(health_url, 1)

        _ = bridge_b.register_project(
            "multi-device-thread-b",
            f"multi-device-b-renewed-{uuid4().hex}",
            project_root,
        )
        replacement_a = _bridge("A", bridge_url)
        _ = replacement_a.restore_project(restart_projects[0])
        _wait_for_connected_devices(health_url, 2)
    finally:
        if replacement_a is not None:
            replacement_a.close()
        bridge_a.close()
        bridge_b.close()
    print("Multi-device smoke passed: 2 connected, 1 isolated restart, 2 restored.")
    return 0


def _bridge(label: str, bridge_url: str) -> RemoteMcpBridge:
    return RemoteMcpBridge(
        RemoteMcpBridgeConfig(
            bridge_url=bridge_url,
            device_id=_required(f"QA_REMOTE_MCP_DEVICE_{label}_ID"),
            device_token=_required(f"QA_REMOTE_MCP_DEVICE_{label}_TOKEN"),
            binding_ttl_seconds=300,
        ),
        log=lambda _message: None,
    )


def _wait_for_connected_devices(health_url: str, expected: int) -> None:
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        health = _health(health_url)
        if health.get("connected_devices") == expected:
            if health.get("configured_devices") != 2:
                raise RuntimeError("Gateway does not report two configured devices.")
            return
        time.sleep(0.1)
    raise RuntimeError(f"Gateway never reported {expected} connected devices.")


def _health(url: str) -> dict[str, object]:
    request = urllib.request.Request(url, headers={"Accept": "application/json"})
    with urllib.request.urlopen(request, timeout=3) as response:
        payload = json.loads(response.read().decode("utf-8"))
    if not isinstance(payload, dict):
        raise TypeError("Gateway health response was not an object.")
    return payload


def _required(name: str) -> str:
    value = os.environ.get(name, "")
    if not value:
        raise RuntimeError(f"{name} is required.")
    return value


if __name__ == "__main__":
    raise SystemExit(main())
