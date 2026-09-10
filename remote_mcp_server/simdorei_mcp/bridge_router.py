from __future__ import annotations

import hmac
import logging
from typing import assert_never

import anyio
from fastapi import APIRouter, WebSocket, WebSocketDisconnect
from pydantic import ValidationError

from remote_mcp_server.simdorei_mcp.bridge_sender import WebSocketBridgeSender
from remote_mcp_server.simdorei_mcp.broker import BindingBroker
from remote_mcp_server.simdorei_mcp.broker_errors import BrokerError
from remote_mcp_server.simdorei_mcp.device_credentials import DeviceAuthenticator
from remote_mcp_server.simdorei_mcp.settings import GatewaySettings
from simdorei_mcp_common.messages import (
    BridgeHello,
    DeviceId,
    GatewayHello,
    ListFilesResult,
    OperationErrorResult,
    ProjectAck,
    ProjectInfoResult,
    ProjectOperationResult,
    ProjectSessionResult,
    ProjectUpsert,
    ReadFileResult,
    WriteFileResult,
    parse_bridge_message,
)

LOGGER = logging.getLogger(__name__)


def create_bridge_router(
    settings: GatewaySettings,
    broker: BindingBroker,
) -> APIRouter:
    router = APIRouter()
    authenticator = DeviceAuthenticator(settings.device_credentials)

    @router.websocket("/bridge")
    async def bridge(socket: WebSocket) -> None:
        expected_device_id = authenticator.authenticate(
            socket.headers.get("authorization", "")
        )
        if expected_device_id is None:
            await socket.close(code=1008, reason="unauthorized")
            return
        await socket.accept()
        sender = WebSocketBridgeSender(socket)
        attached = False
        try:
            first = parse_bridge_message(await socket.receive_text())
            match first:
                case BridgeHello(device_id=device_id) if hmac.compare_digest(
                    str(device_id),
                    str(expected_device_id),
                ):
                    displaced = await broker.attach(device_id, sender)
                    attached = True
                    if displaced is not None:
                        await displaced.close()
                    await sender.send_control(GatewayHello())
                    LOGGER.info("bridge.connected")
                case BridgeHello():
                    await sender.reject(1008, "device mismatch")
                    return
                case (
                    ProjectUpsert()
                    | ProjectInfoResult()
                    | ListFilesResult()
                    | ReadFileResult()
                    | WriteFileResult()
                    | ProjectOperationResult()
                    | ProjectSessionResult()
                    | OperationErrorResult()
                ):
                    await sender.reject(1002, "hello required")
                    return
                case unreachable:  # pyright: ignore[reportUnnecessaryComparison]
                    assert_never(unreachable)
            await _serve_bridge_messages(
                socket,
                sender,
                broker,
                expected_device_id,
            )
        except WebSocketDisconnect:
            LOGGER.info("bridge.disconnected")
        except (ValidationError, BrokerError) as exc:
            LOGGER.warning(
                "bridge.protocol_rejected (%s)",
                type(exc).__name__,
            )
            await sender.reject(1008, "invalid bridge message")
        finally:
            if attached:
                await broker.detach(expected_device_id, sender)

    _ = bridge
    return router


async def _serve_bridge_messages(
    socket: WebSocket,
    sender: WebSocketBridgeSender,
    broker: BindingBroker,
    device_id: DeviceId,
) -> None:
    async with anyio.create_task_group() as resume_tasks:
        try:
            while True:
                message = parse_bridge_message(await socket.receive_text())
                match message:
                    case ProjectUpsert(
                        project_scope=project_scope,
                        binding_id=binding_id,
                    ):
                        await broker.upsert(device_id, sender, message)
                        await sender.send_control(
                            ProjectAck(
                                project_scope=project_scope,
                                binding_id=binding_id,
                            )
                        )
                        _ = resume_tasks.start_soon(
                            _resume_project,
                            broker,
                            device_id,
                            sender,
                            message,
                        )
                    case (
                        ProjectInfoResult()
                        | ListFilesResult()
                        | ReadFileResult()
                        | WriteFileResult()
                        | ProjectOperationResult()
                        | ProjectSessionResult()
                        | OperationErrorResult()
                    ):
                        await broker.complete(device_id, sender, message)
                    case BridgeHello():
                        await sender.reject(1002, "duplicate hello")
                        return
                    case unreachable:  # pyright: ignore[reportUnnecessaryComparison]
                        assert_never(unreachable)
        except WebSocketDisconnect:
            LOGGER.info("bridge.disconnected")
        except (ValidationError, BrokerError) as exc:
            LOGGER.warning(
                "bridge.protocol_rejected (%s)",
                type(exc).__name__,
            )
            await sender.reject(1008, "invalid bridge message")
        finally:
            resume_tasks.cancel_scope.cancel()


async def _resume_project(
    broker: BindingBroker,
    device_id: DeviceId,
    sender: WebSocketBridgeSender,
    project: ProjectUpsert,
) -> None:
    resumed = await broker.resume_project(device_id, sender, project)
    if resumed:
        LOGGER.info("bridge.project_session_resumed")
