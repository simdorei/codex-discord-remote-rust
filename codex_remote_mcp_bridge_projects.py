from __future__ import annotations

from datetime import datetime

from simdorei_mcp_common.messages import ProjectUpsert


def replace_thread_project(
    active: dict[str, ProjectUpsert],
    acknowledged: dict[str, str],
    project: ProjectUpsert,
) -> None:
    """Replace one thread's advertised capability with its newest generation."""
    stale_scopes = tuple(
        scope
        for scope, current in active.items()
        if current.thread_id == project.thread_id and scope != project.project_scope
    )
    for scope in stale_scopes:
        _ = active.pop(scope, None)
        _ = acknowledged.pop(scope, None)
    active[project.project_scope] = project
    _ = acknowledged.pop(project.project_scope, None)


def prune_expired_projects(
    active: dict[str, ProjectUpsert],
    acknowledged: dict[str, str],
    now: datetime,
) -> None:
    expired = {scope for scope, project in active.items() if project.expires_at <= now}
    for scope in expired:
        _ = active.pop(scope, None)
        _ = acknowledged.pop(scope, None)
