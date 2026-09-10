from __future__ import annotations

from concurrent.futures import ThreadPoolExecutor
from datetime import UTC, datetime, timedelta
from pathlib import Path

from fastapi.testclient import TestClient

from remote_mcp_server.simdorei_mcp.app import create_app
from simdorei_mcp_common.connector_contract import PRODUCTION_CONNECTOR_RESOURCE
from simdorei_mcp_common.messages import (
    BridgeHello,
    DeviceId,
    ProjectSessionCommand,
    ProjectSessionResult,
    ProjectUpsert,
    parse_gateway_message,
)
from tests.remote_mcp_oauth_support import (
    BRIDGE_DEVICE_TOKEN,
    authorize,
    oauth_settings,
)


def test_mcp_initialize_and_tool_listing() -> None:
    app = create_app(oauth_settings())
    headers = {
        "Accept": "application/json, text/event-stream",
        "Content-Type": "application/json",
    }
    initialize = {
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": {"name": "integration-test", "version": "1"},
        },
    }

    with TestClient(app, base_url="http://localhost") as client:
        access_token = authorize(client)
        authorized_headers = {
            **headers,
            "Authorization": f"Bearer {access_token}",
        }
        initialized = client.post(
            "/mcp",
            headers=authorized_headers,
            json=initialize,
        )
        tools = client.post(
            "/mcp",
            headers=authorized_headers,
            json={"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}},
        )

    assert initialized.status_code == 200
    assert (
        initialized.json()["result"]["serverInfo"]["name"] == "simdorei-local-project"
    )
    assert tools.status_code == 200
    names = {tool["name"] for tool in tools.json()["result"]["tools"]}
    assert names == {
        "activate_computer_window",
        "capability_inventory",
        "checkpoint_list",
        "checkpoint_restore",
        "checkpoint_show",
        "click_computer_window",
        "close_computer_window",
        "code_search",
        "command_list",
        "command_run",
        "device_info",
        "drag_computer_window",
        "file_apply_patch",
        "file_create",
        "file_read_slice",
        "git_commit",
        "git_push",
        "list_computer_windows",
        "list_devices",
        "list_images",
        "list_project_files",
        "launch_computer_app",
        "press_computer_keys",
        "project_info",
        "project_rules",
        "project_status",
        "read_project_file",
        "repo_diff_summary",
        "repo_status",
        "retrieve_image",
        "save_image",
        "save_image_from_url",
        "screenshot_computer_window",
        "scroll_computer_window",
        "select_device",
        "select_project",
        "set_computer_clipboard",
        "set_working_directory",
        "show_changes",
        "stop_computer_control",
        "terminal_exec",
        "terminal_window_activate",
        "terminal_window_capture",
        "terminal_window_close",
        "terminal_window_interrupt",
        "terminal_window_keys",
        "terminal_window_list",
        "terminal_window_open",
        "terminal_window_type",
        "type_computer_text",
        "write_project_file",
    }


def test_mcp_requires_oauth_and_publishes_discovery(tmp_path: Path) -> None:
    app = create_app(oauth_settings(tmp_path / "oauth.sqlite3"))
    initialize = {
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": {"name": "integration-test", "version": "1"},
        },
    }

    with TestClient(app, base_url="http://localhost") as client:
        protected = client.post(
            "/mcp",
            headers={
                "Accept": "application/json, text/event-stream",
                "Content-Type": "application/json",
            },
            json=initialize,
        )
        resource_metadata = client.get("/.well-known/oauth-protected-resource/mcp")
        server_metadata = client.get("/.well-known/oauth-authorization-server")

    assert protected.status_code == 401
    assert resource_metadata.status_code == 200
    assert resource_metadata.json()["resource"] == "https://simdorei.duckdns.org/mcp"
    assert server_metadata.status_code == 200
    assert server_metadata.json()["registration_endpoint"] == (
        "https://simdorei.duckdns.org/register"
    )


def test_oauth_chat_session_selects_registered_local_project(tmp_path: Path) -> None:
    app = create_app(oauth_settings(tmp_path / "oauth.sqlite3"))
    bridge_headers = {"Authorization": f"Bearer {BRIDGE_DEVICE_TOKEN}"}
    mcp_headers = {
        "Accept": "application/json, text/event-stream",
        "Content-Type": "application/json",
    }

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
            with ThreadPoolExecutor(max_workers=1) as executor:
                pending = executor.submit(
                    client.post,
                    "/mcp",
                    headers={
                        **mcp_headers,
                        "Authorization": f"Bearer {access_token}",
                    },
                    json={
                        "jsonrpc": "2.0",
                        "id": 3,
                        "method": "tools/call",
                        "params": {
                            "name": "select_project",
                            "arguments": {
                                "project_scope": "codex-pro-project-a",
                                "connector_resource": PRODUCTION_CONNECTOR_RESOURCE,
                            },
                            "_meta": {
                                "openai/session": "chat-session-a",
                                "openai/subject": "untrusted-subject",
                            },
                        },
                    },
                )
                command = parse_gateway_message(socket.receive_text())
                assert isinstance(command, ProjectSessionCommand)
                socket.send_text(
                    ProjectSessionResult(
                        request_id=command.request_id
                    ).model_dump_json()
                )
                selected = pending.result(timeout=5)

    assert selected.status_code == 200, selected.text
    result = selected.json()["result"]
    assert result["structuredContent"]["thread_id"] == "thread-a"
    assert result["isError"] is False


def test_oauth_access_token_survives_gateway_restart(tmp_path: Path) -> None:
    database_path = tmp_path / "oauth.sqlite3"
    with TestClient(
        create_app(oauth_settings(database_path)),
        base_url="http://localhost",
    ) as first_client:
        access_token = authorize(first_client)

    with TestClient(
        create_app(oauth_settings(database_path)),
        base_url="http://localhost",
    ) as restarted_client:
        listed = restarted_client.post(
            "/mcp",
            headers={
                "Accept": "application/json, text/event-stream",
                "Content-Type": "application/json",
                "Authorization": f"Bearer {access_token}",
            },
            json={"jsonrpc": "2.0", "id": 4, "method": "tools/list", "params": {}},
        )

    assert listed.status_code == 200, listed.text
    assert listed.json()["result"]["tools"]
