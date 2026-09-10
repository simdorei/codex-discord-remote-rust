from __future__ import annotations

from concurrent.futures import ThreadPoolExecutor

from fastapi.testclient import TestClient

from remote_mcp_server.simdorei_mcp.app import create_app
from remote_mcp_server.simdorei_mcp.capability_inventory import EXPECTED_TOOL_NAMES
from simdorei_mcp_common.connector_contract import PRODUCTION_CONNECTOR_RESOURCE
from simdorei_mcp_common.messages import (
    BridgeHello,
    DeviceId,
    DeviceSessionCommand,
    ProjectSessionResult,
    parse_gateway_message,
)
from tests.remote_mcp_oauth_support import (
    BRIDGE_DEVICE_TOKEN,
    authorize,
    oauth_settings,
)

MCP_HEADERS = {
    "Accept": "application/json, text/event-stream",
    "Content-Type": "application/json",
}


def test_list_devices_returns_the_vps_connected_pc_inventory() -> None:
    # Given
    app = create_app(oauth_settings())
    bridge_headers = {"Authorization": f"Bearer {BRIDGE_DEVICE_TOKEN}"}
    with TestClient(app, base_url="http://localhost") as client:  # noqa: SIM117
        with client.websocket_connect("/bridge", headers=bridge_headers) as socket:
            socket.send_text(
                BridgeHello(
                    protocol_version=10,
                    device_id=DeviceId("device-a"),
                ).model_dump_json()
            )
            _ = parse_gateway_message(socket.receive_text())
            access_token = authorize(client)

            # When
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
                        "name": "list_devices",
                        "arguments": {},
                        "_meta": {"openai/session": "chat-session-a"},
                    },
                },
            )

    # Then
    assert response.status_code == 200, response.text
    result = response.json()["result"]
    assert result["isError"] is False
    assert result["structuredContent"]["devices"] == [
        {"device_id": "device-a", "online": True}
    ]


def test_select_device_binds_the_chat_without_a_project_scope() -> None:
    # Given
    assert "select_device" in EXPECTED_TOOL_NAMES
    app = create_app(oauth_settings())
    bridge_headers = {"Authorization": f"Bearer {BRIDGE_DEVICE_TOKEN}"}
    with TestClient(app, base_url="http://localhost") as client:  # noqa: SIM117
        with client.websocket_connect("/bridge", headers=bridge_headers) as socket:
            socket.send_text(
                BridgeHello(
                    protocol_version=10,
                    device_id=DeviceId("device-a"),
                ).model_dump_json()
            )
            _ = parse_gateway_message(socket.receive_text())
            access_token = authorize(client)
            headers = {
                **MCP_HEADERS,
                "Authorization": f"Bearer {access_token}",
            }

            # When
            with ThreadPoolExecutor(max_workers=1) as executor:
                pending = executor.submit(
                    client.post,
                    "/mcp",
                    headers=headers,
                    json={
                        "jsonrpc": "2.0",
                        "id": 2,
                        "method": "tools/call",
                        "params": {
                            "name": "select_device",
                            "arguments": {
                                "device_id": "device-a",
                                "working_directory": "D:/ERP",
                                "connector_resource": PRODUCTION_CONNECTOR_RESOURCE,
                            },
                            "_meta": {"openai/session": "chat-session-a"},
                        },
                    },
                )
                command = parse_gateway_message(socket.receive_text())
                assert isinstance(command, DeviceSessionCommand)
                socket.send_text(
                    ProjectSessionResult(
                        request_id=command.request_id
                    ).model_dump_json()
                )
                response = pending.result(timeout=5)

            assert "set_working_directory" in EXPECTED_TOOL_NAMES
            with ThreadPoolExecutor(max_workers=1) as executor:
                pending = executor.submit(
                    client.post,
                    "/mcp",
                    headers=headers,
                    json={
                        "jsonrpc": "2.0",
                        "id": 3,
                        "method": "tools/call",
                        "params": {
                            "name": "set_working_directory",
                            "arguments": {"working_directory": "C:/Downloads"},
                            "_meta": {"openai/session": "chat-session-a"},
                        },
                    },
                )
                changed_command = parse_gateway_message(socket.receive_text())
                assert isinstance(changed_command, DeviceSessionCommand)
                socket.send_text(
                    ProjectSessionResult(
                        request_id=changed_command.request_id
                    ).model_dump_json()
                )
                changed_response = pending.result(timeout=5)

            assert "device_info" in EXPECTED_TOOL_NAMES
            info_response = client.post(
                "/mcp",
                headers=headers,
                json={
                    "jsonrpc": "2.0",
                    "id": 4,
                    "method": "tools/call",
                    "params": {
                        "name": "device_info",
                        "arguments": {},
                        "_meta": {"openai/session": "chat-session-a"},
                    },
                },
            )

    # Then
    assert response.status_code == 200, response.text
    result = response.json()["result"]
    assert result["isError"] is False
    assert result["structuredContent"]["device_id"] == "device-a"
    assert result["structuredContent"]["working_directory"] == "D:/ERP"
    assert changed_response.status_code == 200, changed_response.text
    changed_result = changed_response.json()["result"]
    assert changed_result["isError"] is False
    assert changed_result["structuredContent"]["working_directory"] == "C:/Downloads"
    assert info_response.status_code == 200, info_response.text
    info_result = info_response.json()["result"]
    assert info_result["isError"] is False
    assert info_result["structuredContent"]["device_id"] == "device-a"
    assert info_result["structuredContent"]["working_directory"] == "C:/Downloads"
