from __future__ import annotations

from fastapi.testclient import TestClient
from httpx2 import Response

from remote_mcp_server.simdorei_mcp.app import create_app
from remote_mcp_server.simdorei_mcp.oauth_scopes import (
    COMPUTER_CONTROL_REQUIRED_SCOPES,
    COMPUTER_OBSERVE_REQUIRED_SCOPES,
)
from tests.remote_mcp_oauth_support import authorize, oauth_settings

MCP_HEADERS = {
    "Accept": "application/json, text/event-stream",
    "Content-Type": "application/json",
}


def test_exact_advertised_computer_grants_pass_mcp_authorization() -> None:
    app = create_app(oauth_settings())
    with TestClient(app, base_url="http://localhost") as client:
        observe_token = authorize(client, COMPUTER_OBSERVE_REQUIRED_SCOPES)
        screenshot = _call_tool(
            client,
            observe_token,
            "screenshot_computer_window",
            {"window_id": 42},
            "observe-session",
        )
        control_token = authorize(client, COMPUTER_CONTROL_REQUIRED_SCOPES)
        launch = _call_tool(
            client,
            control_token,
            "launch_computer_app",
            {"app": "notepad"},
            "control-session",
        )

    assert screenshot.status_code == 200, screenshot.text
    assert launch.status_code == 200, launch.text
    assert "no active project selection" in screenshot.text
    assert "no active project selection" in launch.text


def _call_tool(
    client: TestClient,
    access_token: str,
    tool_name: str,
    arguments: dict[str, int | str],
    session: str,
) -> Response:
    return client.post(
        "/mcp",
        headers={
            **MCP_HEADERS,
            "Authorization": f"Bearer {access_token}",
        },
        json={
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": tool_name,
                "arguments": arguments,
                "_meta": {"openai/session": session},
            },
        },
    )
