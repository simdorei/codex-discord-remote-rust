from __future__ import annotations

import json
import sys
import urllib.error
import urllib.request
from pathlib import Path
from uuid import uuid4

REPO_ROOT = Path(__file__).resolve().parents[1]
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

import codex_discord_runtime_config as runtime_config
from codex_remote_mcp_bridge import RemoteMcpBridge
from codex_remote_mcp_bridge_config import load_remote_mcp_config

MCP_URL = "https://simdorei.duckdns.org/mcp"


def main() -> int:
    runtime_config.load_local_env(REPO_ROOT / ".env")
    config = load_remote_mcp_config()
    if config is None:
        raise RuntimeError("Remote MCP is disabled in .env.")
    bridge = RemoteMcpBridge(config, log=lambda _: None)
    try:
        scope = f"codex-pro-smoke-{uuid4().hex}"
        ticket = bridge.register_project(
            "remote-mcp-smoke",
            scope,
            REPO_ROOT,
        )
        if ticket.project_scope != scope:
            raise RuntimeError("The gateway acknowledged a different project scope.")
        resource_metadata = _json_get(
            "https://simdorei.duckdns.org/.well-known/oauth-protected-resource/mcp"
        )
        if resource_metadata.get("resource") != MCP_URL:
            raise RuntimeError("OAuth protected-resource metadata is invalid.")
        authorization_metadata = _json_get(
            "https://simdorei.duckdns.org/.well-known/oauth-authorization-server"
        )
        if not isinstance(authorization_metadata.get("authorization_endpoint"), str):
            raise RuntimeError("OAuth authorization metadata is invalid.")
        _expect_oauth_required()
    finally:
        bridge.close()
    print("Remote MCP smoke passed: bridge registration and OAuth discovery are ready.")
    return 0


def _expect_oauth_required() -> None:
    payload = json.dumps(
        {
            "jsonrpc": "2.0",
            "id": uuid4().hex,
            "method": "tools/list",
            "params": {},
        }
    ).encode("utf-8")
    request = urllib.request.Request(
        MCP_URL,
        data=payload,
        headers={
            "Accept": "application/json, text/event-stream",
            "Content-Type": "application/json",
        },
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=20):
            pass
    except urllib.error.HTTPError as exc:
        if exc.code == 401:
            return
        raise
    raise RuntimeError("The MCP endpoint accepted an unauthenticated request.")


def _json_get(url: str) -> dict[str, object]:
    request = urllib.request.Request(url, headers={"Accept": "application/json"})
    with urllib.request.urlopen(request, timeout=20) as response:
        body = json.loads(response.read().decode("utf-8"))
    if not isinstance(body, dict):
        raise TypeError("OAuth metadata response was not an object.")
    return body


if __name__ == "__main__":
    raise SystemExit(main())
