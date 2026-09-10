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
from simdorei_mcp_common.terminal_window_protocol import (
    TerminalWindowCloseOutput,
    TerminalWindowCloseRequest,
    TerminalWindowEntry,
    TerminalWindowListOutput,
    TerminalWindowListRequest,
    TerminalWindowOpenOutput,
    TerminalWindowOpenRequest,
    TerminalWindowOutput,
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
WINDOW_ID = "termwin_0123456789abcdef"


class BridgeSocket(Protocol):
    def receive_text(self) -> str: ...
    def send_text(self, data: str) -> None: ...


def test_terminal_window_tools_round_trip_all_lifecycle_operations() -> None:
    app = create_app(oauth_settings())
    bridge_headers = {"Authorization": f"Bearer {BRIDGE_DEVICE_TOKEN}"}
    with TestClient(app, base_url="http://localhost") as client:  # noqa: SIM117
        with client.websocket_connect("/bridge", headers=bridge_headers) as socket:
            headers = _activate(client, socket)
            entry = _entry()

            opened, open_result = _round_trip(
                client,
                socket,
                headers,
                2,
                "terminal_window_open",
                {"shell": "cmd", "cwd": "tools"},
                TerminalWindowOpenOutput(window=entry),
            )
            listed, list_result = _round_trip(
                client,
                socket,
                headers,
                3,
                "terminal_window_list",
                {},
                TerminalWindowListOutput(windows=(entry,)),
            )
            closed, close_result = _round_trip(
                client,
                socket,
                headers,
                4,
                "terminal_window_close",
                {"terminal_window_id": WINDOW_ID},
                TerminalWindowCloseOutput(terminal_window_id=WINDOW_ID),
            )

    assert isinstance(opened.operation, TerminalWindowOpenRequest)
    assert opened.operation.shell == "cmd"
    assert opened.operation.cwd == "tools"
    assert isinstance(listed.operation, TerminalWindowListRequest)
    assert isinstance(closed.operation, TerminalWindowCloseRequest)
    assert closed.operation.terminal_window_id == WINDOW_ID
    assert _structured(open_result)["window"] == entry.model_dump(mode="json")
    assert _structured(list_result)["windows"] == [entry.model_dump(mode="json")]
    assert _structured(close_result)["closed"] is True


def test_terminal_window_tools_reuse_project_write_oauth_scope() -> None:
    app = create_app(oauth_settings())
    with TestClient(app, base_url="http://localhost") as client:
        token = authorize(client, scopes=(READ_SCOPE, WRITE_SCOPE))
        response = client.post(
            "/mcp",
            headers={**MCP_HEADERS, "Authorization": f"Bearer {token}"},
            json=_tool_call(1, "terminal_window_open", {}),
        )

    assert response.status_code == 200, response.text
    result = cast(dict[str, object], response.json()["result"])
    assert result["isError"] is True
    message = str(result["content"])
    assert "no active project selection" in message
    assert "terminal:execute" not in message


def _activate(
    client: TestClient,
    socket: BridgeSocket,
) -> dict[str, str]:
    socket.send_text(
        BridgeHello(
            protocol_version=10,
            device_id=DeviceId("device-a"),
        ).model_dump_json()
    )
    _ = parse_gateway_message(socket.receive_text())
    socket.send_text(
        ProjectUpsert(
            project_scope="codex-pro-project-terminal-window",
            binding_id="binding-generation-terminal-window",
            thread_id="thread-terminal-window",
            project_name="project-terminal-window",
            expires_at=datetime.now(UTC) + timedelta(minutes=10),
        ).model_dump_json()
    )
    _ = parse_gateway_message(socket.receive_text())
    token = authorize(client)
    headers = {**MCP_HEADERS, "Authorization": f"Bearer {token}"}
    with ThreadPoolExecutor(max_workers=1) as executor:
        pending = executor.submit(
            client.post,
            "/mcp",
            headers=headers,
            json=_tool_call(
                1,
                "select_project",
                {
                    "project_scope": "codex-pro-project-terminal-window",
                    "connector_resource": PRODUCTION_CONNECTOR_RESOURCE,
                },
            ),
        )
        command = parse_gateway_message(socket.receive_text())
        assert isinstance(command, ProjectSessionCommand)
        socket.send_text(
            ProjectSessionResult(request_id=command.request_id).model_dump_json()
        )
        response = pending.result(timeout=5)
    assert response.json()["result"]["isError"] is False
    return headers


def _round_trip(
    client: TestClient,
    socket: BridgeSocket,
    headers: dict[str, str],
    call_id: int,
    name: str,
    arguments: dict[str, object],
    output: TerminalWindowOutput,
) -> tuple[ProjectOperationCommand, dict[str, object]]:
    with ThreadPoolExecutor(max_workers=1) as executor:
        pending = executor.submit(
            client.post,
            "/mcp",
            headers=headers,
            json=_tool_call(call_id, name, arguments),
        )
        command = parse_gateway_message(socket.receive_text())
        assert isinstance(command, ProjectOperationCommand)
        socket.send_text(
            ProjectOperationResult(
                request_id=command.request_id,
                output=output,
            ).model_dump_json()
        )
        response = pending.result(timeout=5)
    assert response.status_code == 200, response.text
    return command, cast(dict[str, object], response.json()["result"])


def _tool_call(
    call_id: int,
    name: str,
    arguments: dict[str, object],
) -> dict[str, object]:
    return {
        "jsonrpc": "2.0",
        "id": call_id,
        "method": "tools/call",
        "params": {
            "name": name,
            "arguments": arguments,
            "_meta": {"openai/session": "chat-session-terminal-window"},
        },
    }


def _entry() -> TerminalWindowEntry:
    return TerminalWindowEntry(
        terminal_window_id=WINDOW_ID,
        window_id=42,
        process_id=84,
        shell="cmd",
        cwd="C:/project/tools",
        title="Codex Pro Terminal",
    )


def _structured(result: dict[str, object]) -> dict[str, object]:
    assert result["isError"] is False
    return cast(dict[str, object], result["structuredContent"])
