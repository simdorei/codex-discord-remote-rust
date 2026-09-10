# pyright: reportUnnecessaryComparison=false
from __future__ import annotations

from datetime import datetime
from typing import assert_never, final

from remote_mcp_server.simdorei_mcp.broker_errors import (
    ActiveBindingMissingError,
    BridgeUnavailableError,
    SessionProjectConflictError,
)
from remote_mcp_server.simdorei_mcp.broker_models import (
    DeviceRouteTarget,
    PendingCall,
    ProjectRouteTarget,
    SessionRoute,
)
from simdorei_mcp_common.messages import DeviceId, RequestId

RouteFailure = ActiveBindingMissingError | BridgeUnavailableError


@final
class BrokerRouteRegistry:  # MUTABLE_OK: caller serializes access with broker lock.
    """Maintains the broker's session-to-thread routing indexes."""

    def __init__(
        self,
        sessions: dict[str, SessionRoute],
        thread_sessions: dict[tuple[DeviceId, str], str],
        pending: dict[RequestId, PendingCall],
    ) -> None:
        self._sessions = sessions
        self._thread_sessions = thread_sessions
        self._pending = pending

    def require_compatible(self, route: SessionRoute, *, now: datetime) -> None:
        current = self._sessions.get(route.session)
        if current is None or current.expires_at <= now:
            return
        if current.subject == route.subject:
            match current.target, route.target:
                case (DeviceRouteTarget(), _) | (_, DeviceRouteTarget()):
                    return
                case ProjectRouteTarget(), ProjectRouteTarget():
                    if (
                        current.device_id == route.device_id
                        and current.thread_id == route.thread_id
                    ):
                        return
                case unreachable:
                    assert_never(unreachable)
        raise SessionProjectConflictError(
            "This ChatGPT conversation is already connected to a different "
            + "Codex thread. Open a new ChatGPT conversation and select there."
        )

    def replace(self, route: SessionRoute) -> None:
        current = self._sessions.get(route.session)
        if current is not None:
            self.remove(current)
        previous_session = self._thread_sessions.get((route.device_id, route.thread_id))
        if previous_session is not None and previous_session != route.session:
            previous = self._sessions.get(previous_session)
            if previous is not None:
                self.remove(previous)
        self._sessions[route.session] = route
        self._thread_sessions[(route.device_id, route.thread_id)] = route.session

    def renew(self, device_id: DeviceId, thread_id: str, expires_at: datetime) -> None:
        session = self._thread_sessions.get((device_id, thread_id))
        route = self._sessions.get(session) if session is not None else None
        if route is not None and expires_at > route.expires_at:
            route.renew(expires_at)

    def remove(
        self,
        route: SessionRoute,
        *,
        failure: RouteFailure | None = None,
    ) -> None:
        if self._sessions.get(route.session) is route:
            del self._sessions[route.session]
        key = (route.device_id, route.thread_id)
        if self._thread_sessions.get(key) == route.session:
            del self._thread_sessions[key]
        self.cancel(
            route,
            failure
            or ActiveBindingMissingError("ChatGPT project selection was replaced"),
        )

    def cancel(self, route: SessionRoute, failure: RouteFailure) -> None:
        for pending in self._pending.values():
            if pending.computer_session_id == route.computer_session_id:
                pending.failure = failure
                pending.event.set()
