from __future__ import annotations

import base64
from concurrent.futures import ThreadPoolExecutor
from datetime import UTC, datetime, timedelta
from typing import Protocol

from fastapi.testclient import TestClient
from pydantic import JsonValue

from remote_mcp_server.simdorei_mcp.app import create_app
from simdorei_mcp_common.connector_contract import PRODUCTION_CONNECTOR_RESOURCE
from simdorei_mcp_common.messages import (
    BridgeHello,
    DeviceId,
    ProjectOperationCommand,
    ProjectOperationResult,
    ProjectSessionCommand,
    ProjectSessionResult,
    ProjectUpsert,
    parse_gateway_message,
)
from simdorei_mcp_common.operation_outputs import (
    FileApplyPatchOutput,
    ImageEntry,
    ImageRetrieveOutput,
    PatchEntry,
)
from simdorei_mcp_common.operation_requests import (
    FileApplyPatchRequest,
    RetrieveImageRequest,
)
from tests.remote_mcp_oauth_support import (
    BRIDGE_DEVICE_TOKEN,
    authorize,
    oauth_settings,
)

PNG_BYTES = b"\x89PNG\r\n\x1a\nroundtrip-image"
MCP_HEADERS = {
    "Accept": "application/json, text/event-stream",
    "Content-Type": "application/json",
}


class BridgeSocket(Protocol):
    def receive_text(self) -> str: ...
    def send_text(self, data: str) -> None: ...


def test_retrieve_image_returns_native_mcp_image_content() -> None:
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
            socket.send_text(
                ProjectUpsert(
                    project_scope="codex-pro-project-a",
                    binding_id="binding-generation-project-a",
                    thread_id="thread-a",
                    project_name="project-a",
                    expires_at=datetime.now(UTC) + timedelta(minutes=10),
                ).model_dump_json()
            )
            _ = parse_gateway_message(socket.receive_text())
            access_token = authorize(client)
            headers = {
                **MCP_HEADERS,
                "Authorization": f"Bearer {access_token}",
            }
            _select_project(client, socket, headers)

            # When
            with ThreadPoolExecutor(max_workers=1) as executor:
                pending = executor.submit(
                    client.post,
                    "/mcp",
                    headers=headers,
                    json=_retrieve_call(),
                )
                command = parse_gateway_message(socket.receive_text())
                assert isinstance(command, ProjectOperationCommand)
                assert isinstance(command.operation, RetrieveImageRequest)
                socket.send_text(
                    ProjectOperationResult(
                        request_id=command.request_id,
                        output=ImageRetrieveOutput(
                            image=ImageEntry(
                                path="assets/test.png",
                                media_type="image/png",
                                size_bytes=len(PNG_BYTES),
                            ),
                            data_base64=base64.b64encode(PNG_BYTES).decode("ascii"),
                        ),
                    ).model_dump_json()
                )
                response = pending.result(timeout=5)

    # Then
    assert response.status_code == 200, response.text
    result = response.json()["result"]
    assert result["content"][0]["type"] == "image"
    assert result["content"][0]["mimeType"] == "image/png"
    assert base64.b64decode(result["content"][0]["data"]) == PNG_BYTES


def test_file_apply_patch_sends_structured_changes_to_the_local_bridge() -> None:
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
            socket.send_text(
                ProjectUpsert(
                    project_scope="codex-pro-project-a",
                    binding_id="binding-generation-project-a",
                    thread_id="thread-a",
                    project_name="project-a",
                    expires_at=datetime.now(UTC) + timedelta(minutes=10),
                ).model_dump_json()
            )
            _ = parse_gateway_message(socket.receive_text())
            access_token = authorize(client)
            headers = {
                **MCP_HEADERS,
                "Authorization": f"Bearer {access_token}",
            }
            _select_project(client, socket, headers)

            with ThreadPoolExecutor(max_workers=1) as executor:
                pending = executor.submit(
                    client.post,
                    "/mcp",
                    headers=headers,
                    json=_file_changes_call(),
                )
                command = parse_gateway_message(socket.receive_text())
                assert isinstance(command, ProjectOperationCommand)
                assert isinstance(command.operation, FileApplyPatchRequest)
                assert command.operation.changes[0].action == "create"
                assert command.operation.changes[0].path == "qa.txt"
                assert command.operation.changes[0].content == "ok\n"
                socket.send_text(
                    ProjectOperationResult(
                        request_id=command.request_id,
                        output=FileApplyPatchOutput(
                            applied=(
                                PatchEntry(
                                    path="qa.txt",
                                    action="add",
                                    added_lines=1,
                                    removed_lines=0,
                                ),
                            ),
                            checkpoint_id="cp_0123456789abcdef",
                        ),
                    ).model_dump_json()
                )
                response = pending.result(timeout=5)

    assert response.status_code == 200, response.text
    result = response.json()["result"]
    assert result["isError"] is False
    assert result["structuredContent"]["applied"][0]["path"] == "qa.txt"


def _select_project(
    client: TestClient,
    socket: BridgeSocket,
    headers: dict[str, str],
) -> None:
    with ThreadPoolExecutor(max_workers=1) as executor:
        pending = executor.submit(
            client.post,
            "/mcp",
            headers=headers,
            json={
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "select_project",
                    "arguments": {
                        "project_scope": "codex-pro-project-a",
                        "connector_resource": PRODUCTION_CONNECTOR_RESOURCE,
                    },
                    "_meta": {"openai/session": "chat-session-a"},
                },
            },
        )
        command = parse_gateway_message(socket.receive_text())
        assert isinstance(command, ProjectSessionCommand)
        socket.send_text(
            ProjectSessionResult(request_id=command.request_id).model_dump_json()
        )
        selected = pending.result(timeout=5)
    assert selected.status_code == 200, selected.text
    assert selected.json()["result"]["isError"] is False


def _retrieve_call() -> dict[str, JsonValue]:
    return {
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "retrieve_image",
            "arguments": {"path": "assets/test.png"},
            "_meta": {"openai/session": "chat-session-a"},
        },
    }


def _file_changes_call() -> dict[str, JsonValue]:
    return {
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "file_apply_patch",
            "arguments": {
                "changes": [
                    {
                        "action": "create",
                        "path": "qa.txt",
                        "content": "ok\n",
                    }
                ]
            },
            "_meta": {"openai/session": "chat-session-a"},
        },
    }
