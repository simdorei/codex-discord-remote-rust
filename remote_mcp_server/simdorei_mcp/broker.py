"""Cohesive project routing and request rendezvous. (# noqa: SIZE_OK)"""

# pyright: reportUnnecessaryComparison=false
from __future__ import annotations

import logging
from collections.abc import Callable
from datetime import UTC, datetime, timedelta
from typing import Final, assert_never, final, override
from uuid import uuid4

import anyio

from remote_mcp_server.simdorei_mcp.broker_errors import (
    ActiveBindingMissingError,
    BindingCodeError,
    BridgeProtocolError,
    BridgeTimeoutError,
    BridgeUnavailableError,
    BrokerError,
)
from remote_mcp_server.simdorei_mcp.broker_idempotency import (
    command_fingerprint,
)
from remote_mcp_server.simdorei_mcp.broker_models import (
    BridgeSender,
    DeviceRouteTarget,
    DormantSessionRoute,
    PendingCall,
    PendingProject,
    ProjectRouteTarget,
    SessionRoute,
)
from remote_mcp_server.simdorei_mcp.broker_registration import (
    disconnected_device_targets,
    stale_registration_targets,
)
from remote_mcp_server.simdorei_mcp.broker_requests import BrokerRequestsMixin
from remote_mcp_server.simdorei_mcp.broker_results import require_project_session_result
from remote_mcp_server.simdorei_mcp.broker_routes import BrokerRouteRegistry
from simdorei_mcp_common.device_messages import (
    DeviceListOutput,
    DeviceSelectionOutput,
    DeviceSummary,
)
from simdorei_mcp_common.leases import RenewableExpiry
from simdorei_mcp_common.messages import (
    BridgeResult,
    DeviceSessionCommand,
    DeviceId,
    GatewayCommand,
    ProjectSelectionOutput,
    ProjectSessionCommand,
    ProjectUpsert,
    RequestId,
)
from simdorei_mcp_common.request_deadlines import (
    GATEWAY_REQUEST_TIMEOUT_SECONDS,
)

logger = logging.getLogger(__name__)
NowFactory = Callable[[], datetime]
RESTART_ROUTE_GRACE_SECONDS: Final = 120
DEVICE_ROUTE_TTL_SECONDS: Final = 1_800
DEVICE_CONTROL_THREAD_ID: Final = "codex-device-control"


@final
class BindingBroker(BrokerRequestsMixin):  # MUTABLE_OK: synchronized routing state.
    """Owns live devices, registered project scopes, and session routes."""

    def __init__(
        self,
        *,
        request_timeout_seconds: float = GATEWAY_REQUEST_TIMEOUT_SECONDS,
        now: NowFactory = lambda: datetime.now(UTC),
    ) -> None:
        self._request_timeout_seconds = request_timeout_seconds
        self._now = now
        self._lock = anyio.Lock()
        self._selection_lock = anyio.Lock()
        self._devices: dict[DeviceId, BridgeSender] = {}
        self._projects: dict[str, PendingProject] = {}
        self._sessions: dict[str, SessionRoute] = {}
        self._thread_sessions: dict[tuple[DeviceId, str], str] = {}
        self._computer_session_generations: dict[tuple[DeviceId, str], int] = {}
        self._dormant_routes: dict[
            tuple[DeviceId, str], DormantSessionRoute
        ] = {}
        self._pending: dict[RequestId, PendingCall] = {}
        self._routes = BrokerRouteRegistry(
            self._sessions,
            self._thread_sessions,
            self._pending,
        )

    @property
    def pending_call_count(self) -> int:
        return len(self._pending)

    @property
    def registered_project_count(self) -> int:
        return len(self._projects)

    @property
    def session_route_count(self) -> int:
        return len(self._sessions)

    @property
    def thread_session_count(self) -> int:
        return len(self._thread_sessions)

    @property
    def dormant_route_count(self) -> int:
        return len(self._dormant_routes)

    async def is_device_connected(self, device_id: DeviceId) -> bool:
        async with self._lock:
            return device_id in self._devices

    async def connected_device_count(self) -> int:
        async with self._lock:
            return len(self._devices)

    async def list_devices(self) -> DeviceListOutput:
        async with self._lock:
            return DeviceListOutput(
                devices=tuple(
                    DeviceSummary(device_id=device_id, online=True)
                    for device_id in sorted(self._devices, key=str.casefold)
                )
            )

    async def select_device(
        self,
        session: str,
        subject: str,
        device_id: DeviceId,
        *,
        working_directory: str,
    ) -> DeviceSelectionOutput:
        async with self._selection_lock:
            async with self._lock:
                sender = self._devices.get(device_id)
                if sender is None:
                    raise BridgeUnavailableError("selected local bridge is disconnected")
                now = self._now()
                generation_key = (device_id, DEVICE_CONTROL_THREAD_ID)
                computer_session_generation = (
                    self._computer_session_generations.get(generation_key, 0) + 1
                )
                self._computer_session_generations[generation_key] = (
                    computer_session_generation
                )
                route = SessionRoute(
                    session=session,
                    device_id=device_id,
                    target=DeviceRouteTarget(
                        thread_id=DEVICE_CONTROL_THREAD_ID,
                        working_directory=working_directory,
                    ),
                    subject=subject,
                    computer_session_id=uuid4().hex,
                    computer_session_generation=computer_session_generation,
                    lease=RenewableExpiry(
                        now + timedelta(seconds=DEVICE_ROUTE_TTL_SECONDS)
                    ),
                )
                self._routes.require_compatible(route, now=now)
                self._routes.replace(route)
            activated = False
            try:
                result = await self._dispatch(
                    route,
                    sender,
                    DeviceSessionCommand(
                        request_id=RequestId(uuid4().hex),
                        thread_id=route.thread_id,
                        computer_session_id=route.computer_session_id,
                        computer_session_generation=computer_session_generation,
                        working_directory=working_directory,
                        expires_at=route.expires_at,
                    ),
                )
                require_project_session_result(result)
                activated = True
            finally:
                if not activated:
                    with anyio.CancelScope(shield=True):
                        async with self._lock:
                            if self._sessions.get(session) is route:
                                self._routes.remove(route)
            return DeviceSelectionOutput(
                device_id=device_id,
                working_directory=working_directory,
                expires_at=route.expires_at,
            )

    async def set_working_directory(
        self,
        session: str,
        subject: str,
        working_directory: str,
    ) -> DeviceSelectionOutput:
        async with self._selection_lock:
            async with self._lock:
                now = self._now()
                self._prune_expired_locked(now)
                current = self._sessions.get(session)
                if (
                    current is None
                    or current.subject != subject
                    or current.expires_at <= now
                ):
                    raise ActiveBindingMissingError(
                        "ChatGPT session has no active device selection"
                    )
                match current.target:
                    case DeviceRouteTarget():
                        pass
                    case ProjectRouteTarget():
                        raise ActiveBindingMissingError(
                            "ChatGPT session is using project mode"
                        )
                    case unreachable:
                        assert_never(unreachable)
                sender = self._devices.get(current.device_id)
                if sender is None:
                    raise BridgeUnavailableError(
                        "selected local bridge is disconnected"
                    )
                generation_key = (current.device_id, DEVICE_CONTROL_THREAD_ID)
                computer_session_generation = (
                    self._computer_session_generations.get(generation_key, 0) + 1
                )
                self._computer_session_generations[generation_key] = (
                    computer_session_generation
                )
                route = SessionRoute(
                    session=session,
                    device_id=current.device_id,
                    target=DeviceRouteTarget(
                        thread_id=DEVICE_CONTROL_THREAD_ID,
                        working_directory=working_directory,
                    ),
                    subject=subject,
                    computer_session_id=uuid4().hex,
                    computer_session_generation=computer_session_generation,
                    lease=RenewableExpiry(
                        now + timedelta(seconds=DEVICE_ROUTE_TTL_SECONDS)
                    ),
                )
                self._routes.replace(route)
            activated = False
            try:
                result = await self._dispatch(
                    route,
                    sender,
                    DeviceSessionCommand(
                        request_id=RequestId(uuid4().hex),
                        thread_id=route.thread_id,
                        computer_session_id=route.computer_session_id,
                        computer_session_generation=computer_session_generation,
                        working_directory=working_directory,
                        expires_at=route.expires_at,
                    ),
                )
                require_project_session_result(result)
                activated = True
            finally:
                if not activated:
                    with anyio.CancelScope(shield=True):
                        async with self._lock:
                            if self._sessions.get(session) is route:
                                self._routes.remove(route)
            return DeviceSelectionOutput(
                device_id=route.device_id,
                working_directory=working_directory,
                expires_at=route.expires_at,
            )

    async def device_info(
        self,
        session: str,
        subject: str,
    ) -> DeviceSelectionOutput:
        async with self._lock:
            now = self._now()
            self._prune_expired_locked(now)
            route = self._sessions.get(session)
            if (
                route is None
                or route.subject != subject
                or route.expires_at <= now
            ):
                raise ActiveBindingMissingError(
                    "ChatGPT session has no active device selection"
                )
            match route.target:
                case DeviceRouteTarget(working_directory=working_directory):
                    return DeviceSelectionOutput(
                        device_id=route.device_id,
                        working_directory=working_directory,
                        expires_at=route.expires_at,
                    )
                case ProjectRouteTarget():
                    raise ActiveBindingMissingError(
                        "ChatGPT session is using project mode"
                    )
                case unreachable:
                    assert_never(unreachable)

    async def attach(
        self,
        device_id: DeviceId,
        sender: BridgeSender,
    ) -> BridgeSender | None:
        async with self._lock:
            displaced = self._devices.get(device_id)
            if displaced is not sender:
                self._disconnect_device(device_id)
            self._devices[device_id] = sender
            return displaced if displaced is not sender else None

    async def detach(self, device_id: DeviceId, sender: BridgeSender) -> None:
        async with self._lock:
            current = self._devices.get(device_id)
            if current is not sender:
                return
            self._disconnect_device(device_id)

    async def upsert(
        self,
        device_id: DeviceId,
        sender: BridgeSender,
        project: ProjectUpsert,
    ) -> None:
        async with self._lock:
            if self._devices.get(device_id) is not sender:
                raise BridgeUnavailableError("local bridge is disconnected")
            now = self._now()
            if project.expires_at <= now:
                raise BindingCodeError("project scope is already expired")
            current = self._projects.get(project.project_scope)
            active_owner = (
                current.device_id
                if current is not None and current.value.expires_at > now
                else None
            )
            if active_owner is None:
                active_owner = next(
                    (
                        dormant_device_id
                        for (dormant_device_id, _), dormant in (
                            self._dormant_routes.items()
                        )
                        if dormant.project_scope == project.project_scope
                        and dormant.resume_until > now
                        and dormant.route.expires_at > now
                    ),
                    None,
                )
            if active_owner is not None and active_owner != device_id:
                raise BindingCodeError("project scope is owned by another device")
            self._prune_expired_locked(now)
            current = self._projects.get(project.project_scope)
            if (
                current is not None
                and current.device_id == device_id
                and current.value.binding_id == project.binding_id
                and current.value.thread_id == project.thread_id
                and current.value.project_name == project.project_name
            ):
                if project.expires_at > current.value.expires_at:
                    self._projects[project.project_scope] = PendingProject(
                        device_id=device_id,
                        value=project,
                    )
                    self._routes.renew(
                        device_id,
                        project.thread_id,
                        project.expires_at,
                    )
                return
            stale_scopes, stale_routes = stale_registration_targets(
                self._projects,
                self._sessions.values(),
                device_id,
                project,
            )
            for scope in stale_scopes:
                del self._projects[scope]
            for route in stale_routes:
                self._routes.remove(route)
            dormant_key = (device_id, project.thread_id)
            dormant = self._dormant_routes.get(dormant_key)
            if dormant is not None and (
                dormant.project_scope != project.project_scope
                or dormant.binding_id != project.binding_id
            ):
                del self._dormant_routes[dormant_key]
            self._projects[project.project_scope] = PendingProject(
                device_id=device_id,
                value=project,
            )
            self._prune_computer_session_generations_locked()

    async def select(
        self,
        session: str,
        subject: str,
        project_scope: str,
    ) -> ProjectSelectionOutput:
        async with self._selection_lock:
            async with self._lock:
                now = self._now()
                self._prune_expired_locked(now)
                pending = self._projects.get(project_scope)
                if pending is None or pending.value.expires_at <= now:
                    raise BindingCodeError("project scope is unavailable or expired")
                generation_key = (pending.device_id, pending.value.thread_id)
                computer_session_generation = (
                    self._computer_session_generations.get(generation_key, 0) + 1
                )
                self._computer_session_generations[generation_key] = (
                    computer_session_generation
                )
                _ = self._dormant_routes.pop(generation_key, None)
                route = SessionRoute(
                    session=session,
                    device_id=pending.device_id,
                    target=ProjectRouteTarget(thread_id=pending.value.thread_id),
                    subject=subject,
                    computer_session_id=uuid4().hex,
                    computer_session_generation=computer_session_generation,
                    lease=RenewableExpiry(pending.value.expires_at),
                )
                self._routes.require_compatible(route, now=now)
                sender = self._devices.get(route.device_id)
                if sender is None:
                    raise BridgeUnavailableError("local bridge is disconnected")
                self._routes.replace(route)
            activated = False
            try:
                result = await self._dispatch(
                    route,
                    sender,
                    ProjectSessionCommand(
                        request_id=RequestId(uuid4().hex),
                        thread_id=route.thread_id,
                        computer_session_id=route.computer_session_id,
                        computer_session_generation=(route.computer_session_generation),
                    ),
                )
                require_project_session_result(result)
                activated = True
            finally:
                if not activated:
                    with anyio.CancelScope(shield=True):
                        async with self._lock:
                            if self._sessions.get(session) is route:
                                self._routes.remove(route)
            return ProjectSelectionOutput(
                project_name=pending.value.project_name,
                thread_id=route.thread_id,
                expires_at=route.expires_at,
            )

    async def resume_project(
        self,
        device_id: DeviceId,
        sender: BridgeSender,
        project: ProjectUpsert,
    ) -> bool:
        async with self._selection_lock:
            async with self._lock:
                now = self._now()
                self._prune_expired_locked(now)
                if self._devices.get(device_id) is not sender:
                    return False
                pending = self._projects.get(project.project_scope)
                key = (device_id, project.thread_id)
                dormant = self._dormant_routes.get(key)
                if (
                    pending is None
                    or pending.device_id != device_id
                    or pending.value != project
                    or dormant is None
                    or dormant.project_scope != project.project_scope
                    or dormant.binding_id != project.binding_id
                    or dormant.resume_until <= now
                ):
                    return False
                del self._dormant_routes[key]
                computer_session_generation = max(
                    self._computer_session_generations.get(key, 0),
                    dormant.route.computer_session_generation,
                ) + 1
                self._computer_session_generations[key] = computer_session_generation
                route = SessionRoute(
                    session=dormant.route.session,
                    device_id=device_id,
                    target=ProjectRouteTarget(thread_id=project.thread_id),
                    subject=dormant.route.subject,
                    computer_session_id=uuid4().hex,
                    computer_session_generation=computer_session_generation,
                    lease=RenewableExpiry(project.expires_at),
                )
                self._routes.replace(route)
            activated = False
            try:
                result = await self._dispatch(
                    route,
                    sender,
                    ProjectSessionCommand(
                        request_id=RequestId(uuid4().hex),
                        thread_id=route.thread_id,
                        computer_session_id=route.computer_session_id,
                        computer_session_generation=(route.computer_session_generation),
                    ),
                )
                require_project_session_result(result)
                activated = True
            except BrokerError:
                return False
            finally:
                if not activated:
                    with anyio.CancelScope(shield=True):
                        async with self._lock:
                            if self._sessions.get(route.session) is route:
                                self._routes.remove(route)
            return True

    async def complete(
        self,
        device_id: DeviceId,
        sender: BridgeSender,
        result: BridgeResult,
    ) -> None:
        async with self._lock:
            if self._devices.get(device_id) is not sender:
                return
            pending = self._pending.get(result.request_id)
            if (
                pending is None
                or pending.device_id != device_id
                or pending.failure is not None
            ):
                return
            pending.result = result
            pending.event.set()

    def _disconnect_device(self, device_id: DeviceId) -> None:
        _ = self._devices.pop(device_id, None)
        now = self._now()
        stale_projects, stale_routes = disconnected_device_targets(
            self._projects,
            self._sessions.values(),
            device_id,
        )
        for route in stale_routes:
            project = next(
                (
                    self._projects[scope].value
                    for scope in stale_projects
                    if self._projects[scope].value.thread_id == route.thread_id
                ),
                None,
            )
            if project is not None and route.expires_at > now:
                self._dormant_routes[(device_id, route.thread_id)] = (
                    DormantSessionRoute(
                        route=route,
                        project_scope=project.project_scope,
                        binding_id=project.binding_id,
                        resume_until=min(
                            route.expires_at,
                            now + timedelta(seconds=RESTART_ROUTE_GRACE_SECONDS),
                        ),
                    )
                )
            del self._sessions[route.session]
            self._routes.cancel(
                route,
                BridgeUnavailableError("selected local bridge is disconnected"),
            )
            _ = self._thread_sessions.pop(
                (route.device_id, route.thread_id),
                None,
            )
        for scope in stale_projects:
            del self._projects[scope]
        self._prune_computer_session_generations_locked()

    def _prune_expired_locked(self, now: datetime) -> None:
        expired_scopes = tuple(
            scope
            for scope, pending in self._projects.items()
            if pending.value.expires_at <= now
        )
        for scope in expired_scopes:
            del self._projects[scope]
        expired_routes = tuple(
            route for route in self._sessions.values() if route.expires_at <= now
        )
        for route in expired_routes:
            self._routes.remove(
                route,
                failure=ActiveBindingMissingError("ChatGPT project selection expired"),
            )
        expired_dormant = tuple(
            key
            for key, dormant in self._dormant_routes.items()
            if dormant.resume_until <= now or dormant.route.expires_at <= now
        )
        for key in expired_dormant:
            del self._dormant_routes[key]
        self._prune_computer_session_generations_locked()

    def _prune_computer_session_generations_locked(self) -> None:
        live_keys = {
            (pending.device_id, pending.value.thread_id)
            for pending in self._projects.values()
        }
        live_keys.update(
            (route.device_id, route.thread_id) for route in self._sessions.values()
        )
        stale_keys = self._computer_session_generations.keys() - live_keys
        for key in stale_keys:
            del self._computer_session_generations[key]

    async def _retire_sender_after_send_timeout(
        self,
        device_id: DeviceId,
        sender: BridgeSender,
    ) -> None:
        with anyio.CancelScope(shield=True):
            async with self._lock:
                if self._devices.get(device_id) is sender:
                    self._disconnect_device(device_id)
        close_grace_seconds = min(0.1, self._request_timeout_seconds)
        try:
            with anyio.move_on_after(close_grace_seconds, shield=True):
                await sender.close()
        except Exception:
            logger.warning(
                "failed to close local bridge after send timeout",
                exc_info=True,
            )

    @override
    async def _dispatch(
        self,
        route: SessionRoute,
        sender: BridgeSender,
        command: GatewayCommand,
    ) -> BridgeResult:
        fingerprint = command_fingerprint(command)
        should_send = False
        async with self._lock:
            if self._sessions.get(route.session) is not route:
                raise ActiveBindingMissingError(
                    "ChatGPT session has no active project selection"
                )
            if self._devices.get(route.device_id) is not sender:
                raise BridgeUnavailableError("selected local bridge is disconnected")
            pending = self._pending.get(command.request_id)
            if pending is None:
                pending = PendingCall(
                    event=anyio.Event(),
                    device_id=route.device_id,
                    computer_session_id=route.computer_session_id,
                    fingerprint=fingerprint,
                )
                self._pending[command.request_id] = pending
                should_send = True
            elif (
                pending.device_id != route.device_id
                or pending.computer_session_id != route.computer_session_id
                or pending.fingerprint != fingerprint
            ):
                raise BridgeProtocolError(
                    "request ID was reused with different command content"
                )
            else:
                pending.waiter_count += 1
        remaining_seconds = (command.deadline_at - datetime.now(UTC)).total_seconds()
        timeout_seconds = min(
            self._request_timeout_seconds,
            max(0.001, remaining_seconds),
        )
        send_completed = not should_send
        try:
            with anyio.fail_after(timeout_seconds):
                if should_send:
                    await sender.send(command)
                    send_completed = True
                await pending.event.wait()
        except TimeoutError as exc:
            if should_send and not send_completed:
                await self._retire_sender_after_send_timeout(
                    route.device_id,
                    sender,
                )
                raise BridgeTimeoutError("local bridge send timed out") from exc
            raise BridgeTimeoutError("local bridge response timed out") from exc
        finally:
            with anyio.CancelScope(shield=True):
                async with self._lock:
                    pending.waiter_count -= 1
                    if (
                        pending.waiter_count == 0
                        and self._pending.get(command.request_id) is pending
                    ):
                        _ = self._pending.pop(command.request_id, None)
        if pending.failure is not None:
            raise pending.failure
        if pending.result is None:
            raise BridgeUnavailableError("local bridge returned no result")
        return pending.result

    @override
    async def _route(
        self,
        session: str,
        subject: str,
    ) -> tuple[SessionRoute, BridgeSender]:
        async with self._lock:
            now = self._now()
            self._prune_expired_locked(now)
            route = self._sessions.get(session)
            if route is None or route.expires_at <= now or route.subject != subject:
                raise ActiveBindingMissingError(
                    "ChatGPT session has no active project selection"
                )
            sender = self._devices.get(route.device_id)
            if sender is None:
                raise BridgeUnavailableError("selected local bridge is disconnected")
            return route, sender
