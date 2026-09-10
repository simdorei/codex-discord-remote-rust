from __future__ import annotations

from collections.abc import Iterable, Mapping

from remote_mcp_server.simdorei_mcp.broker_models import PendingProject, SessionRoute
from simdorei_mcp_common.messages import DeviceId, ProjectUpsert


def disconnected_device_targets(
    projects: Mapping[str, PendingProject],
    sessions: Iterable[SessionRoute],
    device_id: DeviceId,
) -> tuple[tuple[str, ...], tuple[SessionRoute, ...]]:
    """Find every registration and session owned by a disconnected bridge."""
    scopes = tuple(
        scope for scope, project in projects.items() if project.device_id == device_id
    )
    routes = tuple(route for route in sessions if route.device_id == device_id)
    return scopes, routes


def stale_registration_targets(
    projects: Mapping[str, PendingProject],
    sessions: Iterable[SessionRoute],
    device_id: DeviceId,
    project: ProjectUpsert,
) -> tuple[tuple[str, ...], tuple[SessionRoute, ...]]:
    """Find older capabilities and sessions replaced by one fresh registration."""
    scopes = tuple(
        scope
        for scope, pending in projects.items()
        if pending.device_id == device_id
        and pending.value.thread_id == project.thread_id
        and scope != project.project_scope
    )
    routes = tuple(
        route
        for route in sessions
        if route.device_id == device_id and route.thread_id == project.thread_id
    )
    return scopes, routes
