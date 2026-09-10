from __future__ import annotations

from concurrent.futures import ThreadPoolExecutor
from datetime import UTC, datetime, timedelta
from typing import Protocol, cast

from fastapi.testclient import TestClient

from remote_mcp_server.simdorei_mcp.app import create_app
from remote_mcp_server.simdorei_mcp.oauth_scopes import READ_SCOPE, WRITE_SCOPE
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
from simdorei_mcp_common.terminal_protocol import (
    TerminalExecOutput,
    TerminalExecRequest,
    TerminalExecutionReceipt,
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


class BridgeSocket(Protocol):
    def receive_text(self) -> str: ...
    def send_text(self, data: str) -> None: ...


def test_terminal_tool_round_trips_typed_request_and_receipt() -> None:
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
                    project_scope="codex-pro-project-terminal",
                    binding_id="binding-generation-terminal",
                    thread_id="thread-terminal",
                    project_name="project-terminal",
                    expires_at=datetime.now(UTC) + timedelta(minutes=10),
                ).model_dump_json()
            )
            _ = parse_gateway_message(socket.receive_text())
            access_token = authorize(client)
            headers = {**MCP_HEADERS, "Authorization": f"Bearer {access_token}"}
            _select_project(client, socket, headers)

            with ThreadPoolExecutor(max_workers=1) as executor:
                pending = executor.submit(
                    client.post,
                    "/mcp",
                    headers=headers,
                    json=_terminal_call(),
                )
                command = parse_gateway_message(socket.receive_text())
                assert isinstance(command, ProjectOperationCommand)
                assert isinstance(command.operation, TerminalExecRequest)
                assert command.operation.command == "Write-Output terminal-ok"
                assert command.operation.cwd == "C:/Windows/Temp"
                assert command.operation.cancel_previous is True
                socket.send_text(
                    ProjectOperationResult(
                        request_id=command.request_id,
                        output=_terminal_output(),
                    ).model_dump_json()
                )
                response = pending.result(timeout=5)

    assert response.status_code == 200, response.text
    payload = cast(dict[str, object], response.json())
    result = cast(dict[str, object], payload["result"])
    assert result["isError"] is False
    structured = cast(dict[str, object], result["structuredContent"])
    receipt = cast(dict[str, object], structured["receipt"])
    assert structured["stdout"] == "terminal-ok\n"
    assert receipt["command_digest"] == "a" * 64


def test_terminal_tool_reuses_project_write_oauth_scope() -> None:
    app = create_app(oauth_settings())
    with TestClient(app, base_url="http://localhost") as client:
        token = authorize(client, scopes=(READ_SCOPE, WRITE_SCOPE))
        response = client.post(
            "/mcp",
            headers={**MCP_HEADERS, "Authorization": f"Bearer {token}"},
            json=_terminal_call(),
        )

    assert response.status_code == 200, response.text
    payload = cast(dict[str, object], response.json())
    result = cast(dict[str, object], payload["result"])
    assert result["isError"] is True
    content = cast(list[dict[str, object]], result["content"])
    message = str(content[0]["text"])
    assert "no active project selection" in message
    assert "terminal:execute" not in message


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
                        "project_scope": "codex-pro-project-terminal",
                        "connector_resource": PRODUCTION_CONNECTOR_RESOURCE,
                    },
                    "_meta": {"openai/session": "chat-session-terminal"},
                },
            },
        )
        command = parse_gateway_message(socket.receive_text())
        assert isinstance(command, ProjectSessionCommand)
        socket.send_text(
            ProjectSessionResult(request_id=command.request_id).model_dump_json()
        )
        response = pending.result(timeout=5)
    assert response.status_code == 200, response.text
    assert response.json()["result"]["isError"] is False


def _terminal_call() -> dict[str, object]:
    return {
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "terminal_exec",
            "arguments": {
                "command": "Write-Output terminal-ok",
                "shell": "powershell",
                "cwd": "C:/Windows/Temp",
                "timeout_seconds": 90,
                "cancel_previous": True,
            },
            "_meta": {"openai/session": "chat-session-terminal"},
        },
    }


def _terminal_output() -> TerminalExecOutput:
    receipt = TerminalExecutionReceipt(
        receipt_id="tr_0123456789abcdef",
        terminal_id="term_0123456789abcdef",
        command_digest="a" * 64,
        shell="powershell",
        cwd_scope="external_absolute",
        exit_code=0,
        stdout_bytes=12,
        stderr_bytes=0,
        duration_ms=25,
        timed_out=False,
        cancelled=False,
        truncated=False,
    )
    return TerminalExecOutput(
        terminal_id=receipt.terminal_id,
        process_id=123,
        exit_code=0,
        stdout="terminal-ok\n",
        stderr="",
        cwd="C:/Windows/Temp",
        duration_ms=25,
        timed_out=False,
        cancelled=False,
        truncated=False,
        receipt=receipt,
    )
