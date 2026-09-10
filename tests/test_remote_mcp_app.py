from __future__ import annotations

from builtins import BaseExceptionGroup
from contextlib import asynccontextmanager
from datetime import UTC, datetime, timedelta
from typing import assert_never, final

import anyio
import anyio.lowlevel
import pytest
from fastapi.testclient import TestClient
from pydantic import ValidationError
from starlette.websockets import WebSocketDisconnect

from remote_mcp_server.simdorei_mcp.app import (
    _close_preserving_lifespan,
    create_app,
)
from remote_mcp_server.simdorei_mcp.bridge_sender import WebSocketBridgeSender
from remote_mcp_server.simdorei_mcp.settings import GatewaySettings
from simdorei_mcp_common.messages import (
    BridgeHello,
    DeviceId,
    GatewayHello,
    ListFilesCommand,
    ProjectAck,
    ProjectInfoCommand,
    ProjectOperationCommand,
    ProjectSessionCommand,
    ProjectUpsert,
    ReadFileCommand,
    WriteFileCommand,
    parse_gateway_message,
)

TOKEN_A = "a" * 40
TOKEN_B = "b" * 40


def _settings() -> GatewaySettings:
    return GatewaySettings.model_validate(
        {
            "device_credentials": {
                "version": 1,
                "devices": [
                    {"device_id": "device-a", "token": TOKEN_A},
                    {"device_id": "device-b", "token": TOKEN_B},
                ],
            },
            "public_base_url": "https://simdorei.duckdns.org",
            "owner_token": "owner-secret-12345678901234567890",
        }
    )


def test_bridge_accepts_authenticated_project_registration() -> None:
    app = create_app(_settings())
    headers = {"Authorization": f"Bearer {TOKEN_A}"}

    with (
        TestClient(app) as client,
        client.websocket_connect("/bridge", headers=headers) as socket,
    ):
        socket.send_text(
            BridgeHello(
                protocol_version=10,
                device_id=DeviceId("device-a"),
            ).model_dump_json()
        )
        hello = parse_gateway_message(socket.receive_text())
        socket.send_text(
            ProjectUpsert(
                project_scope="codex-pro-project-a",
                binding_id="binding-generation-project-a",
                thread_id="thread-a",
                project_name="project-a",
                expires_at=datetime.now(UTC) + timedelta(minutes=10),
            ).model_dump_json()
        )
        project = parse_gateway_message(socket.receive_text())

    match hello:
        case GatewayHello():
            pass
        case (
            ProjectAck()
            | ProjectInfoCommand()
            | ProjectOperationCommand()
            | ProjectSessionCommand()
            | ListFilesCommand()
            | ReadFileCommand()
            | WriteFileCommand()
        ):
            raise AssertionError(f"unexpected message: {hello.type}")
        case unreachable:
            assert_never(unreachable)
    match project:
        case ProjectAck(binding_id=binding_id):
            assert binding_id == "binding-generation-project-a"
        case (
            GatewayHello()
            | ProjectInfoCommand()
            | ProjectOperationCommand()
            | ProjectSessionCommand()
            | ListFilesCommand()
            | ReadFileCommand()
            | WriteFileCommand()
        ):
            raise AssertionError(f"unexpected message: {project.type}")
        case unreachable:
            assert_never(unreachable)


def test_bridge_requires_hello_before_project_registration() -> None:
    app = create_app(_settings())
    headers = {"Authorization": f"Bearer {TOKEN_A}"}

    with (
        TestClient(app) as client,
        client.websocket_connect("/bridge", headers=headers) as socket,
    ):
        socket.send_text(
            ProjectUpsert(
                project_scope="codex-pro-project-a",
                binding_id="binding-generation-project-a",
                thread_id="thread-a",
                project_name="project-a",
                expires_at=datetime.now(UTC) + timedelta(minutes=10),
            ).model_dump_json()
        )
        with pytest.raises(WebSocketDisconnect) as closed:
            _ = socket.receive_text()

    assert closed.value.code == 1002


def test_health_reports_bridge_disconnected_before_local_bridge_attaches() -> None:
    app = create_app(_settings())

    with TestClient(app) as client:
        response = client.get("/healthz")

    assert response.status_code == 200
    assert response.json()["ok"] is True
    assert response.json()["upstream_ready"] is False
    assert response.json()["configured_devices"] == 2
    assert response.json()["connected_devices"] == 0


def test_bridge_protocol_rejects_version_one_hello() -> None:
    with pytest.raises(ValidationError):
        BridgeHello.model_validate(
            {
                "type": "hello",
                "protocol_version": 1,
                "device_id": "device-a",
            }
        )


def test_replacing_a_bridge_connection_closes_the_displaced_socket() -> None:
    app = create_app(_settings())
    headers = {"Authorization": f"Bearer {TOKEN_A}"}

    with (
        TestClient(app) as client,
        client.websocket_connect("/bridge", headers=headers) as first,
    ):
        first.send_text(
            BridgeHello(
                protocol_version=10,
                device_id=DeviceId("device-a"),
            ).model_dump_json()
        )
        assert isinstance(parse_gateway_message(first.receive_text()), GatewayHello)

        with client.websocket_connect("/bridge", headers=headers) as replacement:
            replacement.send_text(
                BridgeHello(
                    protocol_version=10,
                    device_id=DeviceId("device-a"),
                ).model_dump_json()
            )
            assert isinstance(
                parse_gateway_message(replacement.receive_text()),
                GatewayHello,
            )
            with pytest.raises(WebSocketDisconnect) as closed:
                _ = first.receive_text()

    assert closed.value.code == 1012


@final
class _BlockingGatewaySocket:
    def __init__(self) -> None:
        self.send_started = anyio.Event()
        self.release_send = anyio.Event()
        self.close_called = anyio.Event()
        self.sending = False
        self.close_overlapped_send = False
        self.close_code = 0
        self.close_reason = ""

    async def send_text(self, data: str) -> None:
        _ = data
        self.sending = True
        self.send_started.set()
        await self.release_send.wait()
        self.sending = False

    async def close(self, code: int = 1000, reason: str | None = None) -> None:
        self.close_overlapped_send = self.sending
        self.close_code = code
        self.close_reason = reason or ""
        self.close_called.set()


def test_gateway_sender_serializes_protocol_close_after_an_inflight_send() -> None:
    async def scenario() -> None:
        socket = _BlockingGatewaySocket()
        sender = WebSocketBridgeSender(socket)

        async with anyio.create_task_group() as tasks:
            tasks.start_soon(sender.send_control, GatewayHello())
            await socket.send_started.wait()
            tasks.start_soon(sender.reject, 1002, "duplicate hello")
            await anyio.lowlevel.checkpoint()
            assert not socket.close_called.is_set()
            socket.release_send.set()

        assert socket.close_called.is_set()
        assert socket.close_overlapped_send is False
        assert socket.close_code == 1002
        assert socket.close_reason == "duplicate hello"

    anyio.run(scenario)


def test_lifespan_preserves_session_and_oauth_close_failures() -> None:
    class SessionFailure(RuntimeError):
        pass

    class OAuthFailure(RuntimeError):
        pass

    @asynccontextmanager
    async def failing_session():
        yield
        raise SessionFailure("session close failed")

    async def fail_oauth_close() -> None:
        raise OAuthFailure("oauth close failed")

    async def scenario() -> None:
        with pytest.raises(BaseExceptionGroup) as captured:
            async with _close_preserving_lifespan(
                failing_session(),
                fail_oauth_close,
            ):
                pass
        assert tuple(type(error) for error in captured.value.exceptions) == (
            SessionFailure,
            OAuthFailure,
        )

    anyio.run(scenario)


def test_duplicate_hello_closes_with_protocol_error() -> None:
    app = create_app(_settings())
    headers = {"Authorization": f"Bearer {TOKEN_A}"}

    with (
        TestClient(app) as client,
        client.websocket_connect("/bridge", headers=headers) as socket,
    ):
        hello = BridgeHello(
            protocol_version=10,
            device_id=DeviceId("device-a"),
        ).model_dump_json()
        socket.send_text(hello)
        assert isinstance(parse_gateway_message(socket.receive_text()), GatewayHello)

        socket.send_text(hello)
        with pytest.raises(WebSocketDisconnect) as closed:
            _ = socket.receive_text()

    assert closed.value.code == 1002


def test_two_devices_connect_and_register_projects_independently() -> None:
    app = create_app(_settings())
    headers_a = {"Authorization": f"Bearer {TOKEN_A}"}
    headers_b = {"Authorization": f"Bearer {TOKEN_B}"}

    with (
        TestClient(app) as client,
        client.websocket_connect("/bridge", headers=headers_a) as socket_a,
        client.websocket_connect("/bridge", headers=headers_b) as socket_b,
    ):
        socket_a.send_text(
            BridgeHello(
                protocol_version=10,
                device_id=DeviceId("device-a"),
            ).model_dump_json()
        )
        socket_b.send_text(
            BridgeHello(
                protocol_version=10,
                device_id=DeviceId("device-b"),
            ).model_dump_json()
        )
        assert isinstance(parse_gateway_message(socket_a.receive_text()), GatewayHello)
        assert isinstance(parse_gateway_message(socket_b.receive_text()), GatewayHello)
        health = client.get("/healthz").json()
        assert health["configured_devices"] == 2
        assert health["connected_devices"] == 2
        assert health["upstream_ready"] is True

        socket_b.send_text(
            ProjectUpsert(
                project_scope="codex-pro-project-b",
                binding_id="binding-generation-project-b",
                thread_id="thread-b",
                project_name="project-b",
                expires_at=datetime.now(UTC) + timedelta(minutes=10),
            ).model_dump_json()
        )
        assert isinstance(parse_gateway_message(socket_b.receive_text()), ProjectAck)


def test_bridge_token_cannot_claim_another_device_id() -> None:
    app = create_app(_settings())
    headers = {"Authorization": f"Bearer {TOKEN_A}"}

    with (
        TestClient(app) as client,
        client.websocket_connect("/bridge", headers=headers) as socket,
    ):
        socket.send_text(
            BridgeHello(
                protocol_version=10,
                device_id=DeviceId("device-b"),
            ).model_dump_json()
        )
        with pytest.raises(WebSocketDisconnect) as closed:
            _ = socket.receive_text()

    assert closed.value.code == 1008


def test_unknown_bridge_token_is_rejected_before_hello() -> None:
    app = create_app(_settings())
    headers = {"Authorization": f"Bearer {'z' * 40}"}

    with (
        TestClient(app) as client,
        pytest.raises(WebSocketDisconnect) as closed,
        client.websocket_connect("/bridge", headers=headers),
    ):
        pass

    assert closed.value.code == 1008
