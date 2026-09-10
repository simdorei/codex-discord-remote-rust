from __future__ import annotations

from fastapi.testclient import TestClient

from remote_mcp_server.simdorei_mcp.app import create_app
from remote_mcp_server.simdorei_mcp.settings import GatewaySettings
from tests.remote_mcp_oauth_support import authorize, oauth_settings

V12_RESOURCE = "https://simdorei.duckdns.org/v12/mcp"
MCP_HEADERS = {
    "Accept": "application/json, text/event-stream",
    "Content-Type": "application/json",
}


def test_select_project_schema_requires_connector_resource() -> None:
    app = create_app(oauth_settings())

    with TestClient(app, base_url="http://localhost") as client:
        access_token = authorize(client)
        response = client.post(
            "/mcp",
            headers={
                **MCP_HEADERS,
                "Authorization": f"Bearer {access_token}",
            },
            json={"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}},
        )

    assert response.status_code == 200, response.text
    select_project = next(
        tool
        for tool in response.json()["result"]["tools"]
        if tool["name"] == "select_project"
    )
    assert "connector_resource" in select_project["inputSchema"]["required"]


def test_v12_gateway_rejects_select_project_before_broker_lookup() -> None:
    settings = GatewaySettings.model_validate(
        {
            **oauth_settings().model_dump(),
            "public_base_url": "https://simdorei.duckdns.org/v12",
        }
    )
    app = create_app(settings)

    with TestClient(app, base_url="http://localhost") as client:
        access_token = authorize(client, resource=V12_RESOURCE)
        response = client.post(
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
                    "name": "select_project",
                    "arguments": {
                        "project_scope": "codex-pro-project-a",
                        "connector_resource": V12_RESOURCE,
                    },
                    "_meta": {"openai/session": "chat-session-a"},
                },
            },
        )

    assert response.status_code == 200, response.text
    result = response.json()["result"]
    assert result["isError"] is True
    assert "OAuth connector mismatch" in result["content"][0]["text"]
