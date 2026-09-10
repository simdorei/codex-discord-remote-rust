from __future__ import annotations

from collections.abc import Mapping
from concurrent.futures import ThreadPoolExecutor
from datetime import UTC, datetime, timedelta
from typing import Protocol, cast

from fastapi.testclient import TestClient

from remote_mcp_server.simdorei_mcp.app import create_app
from remote_mcp_server.simdorei_mcp.oauth_scopes import (
    READ_SCOPE,
    WRITE_SCOPE,
)
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
from simdorei_mcp_common.terminal_window_interaction_protocol import (
    TerminalWindowAction,
    TerminalWindowActionOutput,
    TerminalWindowActionReceipt,
    TerminalWindowActivateRequest,
    TerminalWindowCaptureOutput,
    TerminalWindowCaptureRequest,
    TerminalWindowInteractionOutput,
    TerminalWindowInterruptRequest,
    TerminalWindowKeysRequest,
    TerminalWindowRect,
    TerminalWindowTypeRequest,
)
from simdorei_mcp_common.terminal_window_protocol import TerminalWindowEntry
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
OBSERVATION_ID = "twobs_0123456789abcdef"


class BridgeSocket(Protocol):
    def receive_text(self) -> str: ...
    def send_text(self, data: str) -> None: ...


def test_interaction_tools_round_trip_all_five_operations() -> None:
    app = create_app(oauth_settings())
    bridge_headers = {"Authorization": f"Bearer {BRIDGE_DEVICE_TOKEN}"}
    with TestClient(app, base_url="http://localhost") as client:  # noqa: SIM117
        with client.websocket_connect("/bridge", headers=bridge_headers) as socket:
            headers = _activate(client, socket)
            capture = _capture_output()
            calls: tuple[
                tuple[
                    str,
                    Mapping[str, object],
                    TerminalWindowInteractionOutput,
                    type[object],
                ],
                ...,
            ] = (
                (
                    "terminal_window_capture",
                    {"terminal_window_id": WINDOW_ID},
                    capture,
                    TerminalWindowCaptureRequest,
                ),
                (
                    "terminal_window_activate",
                    {"terminal_window_id": WINDOW_ID},
                    _action_output("activate"),
                    TerminalWindowActivateRequest,
                ),
                (
                    "terminal_window_type",
                    {
                        "terminal_window_id": WINDOW_ID,
                        "observation_id": OBSERVATION_ID,
                        "text": "echo hello",
                    },
                    _action_output("type"),
                    TerminalWindowTypeRequest,
                ),
                (
                    "terminal_window_keys",
                    {
                        "terminal_window_id": WINDOW_ID,
                        "observation_id": OBSERVATION_ID,
                        "keys": ["CTRL", "L"],
                    },
                    _action_output("keys"),
                    TerminalWindowKeysRequest,
                ),
                (
                    "terminal_window_interrupt",
                    {
                        "terminal_window_id": WINDOW_ID,
                        "observation_id": OBSERVATION_ID,
                    },
                    _action_output("interrupt"),
                    TerminalWindowInterruptRequest,
                ),
            )
            for call_id, (name, arguments, output, request_type) in enumerate(
                calls,
                start=2,
            ):
                command, result = _round_trip(
                    client, socket, headers, call_id, name, arguments, output
                )
                assert isinstance(command.operation, request_type)
                assert result["isError"] is False


def test_interaction_tools_require_write_scope() -> None:
    app = create_app(oauth_settings())
    with TestClient(app, base_url="http://localhost") as client:
        token = authorize(client, scopes=(READ_SCOPE,))
        response = client.post(
            "/mcp",
            headers={**MCP_HEADERS, "Authorization": f"Bearer {token}"},
            json=_tool_call(
                1,
                "terminal_window_capture",
                {"terminal_window_id": WINDOW_ID},
            ),
        )

    result = cast(dict[str, object], response.json()["result"])
    assert result["isError"] is True
    assert "files:write" in str(result["content"])


def test_repeated_chatgpt_request_id_gets_fresh_local_window_commands() -> None:
    app = create_app(oauth_settings())
    bridge_headers = {"Authorization": f"Bearer {BRIDGE_DEVICE_TOKEN}"}
    with TestClient(app, base_url="http://localhost") as client:  # noqa: SIM117
        with client.websocket_connect("/bridge", headers=bridge_headers) as socket:
            headers = _activate(client, socket)
            first, _ = _round_trip(
                client,
                socket,
                headers,
                2,
                "terminal_window_capture",
                {"terminal_window_id": WINDOW_ID},
                _capture_output(),
            )
            second, _ = _round_trip(
                client,
                socket,
                headers,
                2,
                "terminal_window_capture",
                {"terminal_window_id": WINDOW_ID},
                _capture_output(),
            )

    assert first.request_id != second.request_id


def _activate(client: TestClient, socket: BridgeSocket) -> dict[str, str]:
    socket.send_text(
        BridgeHello(
            protocol_version=10,
            device_id=DeviceId("device-a"),
        ).model_dump_json()
    )
    _ = parse_gateway_message(socket.receive_text())
    socket.send_text(
        ProjectUpsert(
            project_scope="codex-pro-project-terminal-interact",
            binding_id="binding-generation-terminal-interact",
            thread_id="thread-terminal-interact",
            project_name="project-terminal-interact",
            expires_at=datetime.now(UTC) + timedelta(minutes=10),
        ).model_dump_json()
    )
    _ = parse_gateway_message(socket.receive_text())
    token = authorize(client, scopes=(READ_SCOPE, WRITE_SCOPE))
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
                    "project_scope": "codex-pro-project-terminal-interact",
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
    arguments: Mapping[str, object],
    output: TerminalWindowInteractionOutput,
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
    return command, cast(dict[str, object], response.json()["result"])


def _tool_call(
    call_id: int,
    name: str,
    arguments: Mapping[str, object],
) -> dict[str, object]:
    return {
        "jsonrpc": "2.0",
        "id": call_id,
        "method": "tools/call",
        "params": {
            "name": name,
            "arguments": dict(arguments),
            "_meta": {"openai/session": "chat-session-terminal-interact"},
        },
    }


def _capture_output() -> TerminalWindowCaptureOutput:
    return TerminalWindowCaptureOutput(
        window=_entry(),
        observation_id=OBSERVATION_ID,
        identity_digest="a" * 64,
        rect=TerminalWindowRect(left=1, top=2, width=640, height=480),
        data_base64="aGVsbG8gd29ybGQ=",
        captured_at=datetime.now(UTC),
    )


def _action_output(action: TerminalWindowAction) -> TerminalWindowActionOutput:
    observation_id = None if action == "activate" else OBSERVATION_ID
    keys = ("CTRL", "C") if action == "interrupt" else ()
    if action == "keys":
        keys = ("CTRL", "L")
    return TerminalWindowActionOutput(
        window=_entry(),
        receipt=TerminalWindowActionReceipt(
            receipt_id="twrcpt_0123456789abcdef",
            terminal_window_id=WINDOW_ID,
            observation_id=observation_id,
            identity_digest="b" * 64,
            action=action,
            unicode_chars=10 if action == "type" else 0,
            keys=keys,
            activated=action == "activate",
            completed_at=datetime.now(UTC),
        ),
    )


def _entry() -> TerminalWindowEntry:
    return TerminalWindowEntry(
        terminal_window_id=WINDOW_ID,
        window_id=42,
        process_id=84,
        shell="cmd",
        cwd="C:/project",
        title="Codex Pro Terminal",
    )
