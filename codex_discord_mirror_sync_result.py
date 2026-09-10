from __future__ import annotations

from pathlib import Path

type MirrorCleanupValue = int | list[str]
type MirrorCleanupResult = dict[str, MirrorCleanupValue]


def empty_mirror_cleanup_result() -> MirrorCleanupResult:
    return {
        "deleted": 0,
        "missing": 0,
        "skipped": 0,
        "failed": 0,
        "errors": [],
    }


def cleanup_count(result: MirrorCleanupResult, key: str) -> int:
    value = result.get(key, 0)
    return value if isinstance(value, int) else 0


def cleanup_errors(result: MirrorCleanupResult) -> list[str]:
    value = result.get("errors", [])
    if not isinstance(value, list):
        return []
    return value


def mirror_sync_error_lines(title: str, errors: list[str]) -> list[str]:
    if not errors:
        return []
    return ["", title, *[f"- {error}" for error in errors]]


def format_mirror_sync_result(
    *,
    cleanup_scope: str,
    project_count: int,
    mirrored: int,
    stale_thread_count: int,
    stale_project_count: int,
    stale_cleanup: MirrorCleanupResult,
    orphan_cleanup: MirrorCleanupResult,
    stale_project_cleanup: MirrorCleanupResult,
    db_path: Path,
    app_server_unavailable_count: int = 0,
) -> str:
    lines = [
        "Mirror sync complete.",
        "`rec archive` threads are not removed by sync.",
        "Archive those Codex threads first, then run sync.",
        f"cleanup_scope: {cleanup_scope}",
        f"projects: {project_count}",
        f"threads: {mirrored}",
        *(
            [f"app_server_unavailable_threads: {app_server_unavailable_count}"]
            if app_server_unavailable_count
            else []
        ),
        f"stale_threads_removed: {stale_thread_count}",
        f"stale_discord_threads_deleted: {cleanup_count(stale_cleanup, 'deleted')}",
        f"stale_discord_threads_missing: {cleanup_count(stale_cleanup, 'missing')}",
        f"stale_discord_threads_failed: {cleanup_count(stale_cleanup, 'failed')}",
        f"orphan_discord_threads_deleted: {cleanup_count(orphan_cleanup, 'deleted')}",
        f"orphan_discord_threads_skipped: {cleanup_count(orphan_cleanup, 'skipped')}",
        f"orphan_discord_threads_failed: {cleanup_count(orphan_cleanup, 'failed')}",
        f"stale_projects_removed: {stale_project_count}",
        f"stale_project_channels_deleted: {cleanup_count(stale_project_cleanup, 'deleted')}",
        f"stale_project_channels_missing: {cleanup_count(stale_project_cleanup, 'missing')}",
        f"stale_project_channels_skipped: {cleanup_count(stale_project_cleanup, 'skipped')}",
        f"stale_project_channels_failed: {cleanup_count(stale_project_cleanup, 'failed')}",
        f"database: {db_path}",
        *mirror_sync_error_lines("Discord stale cleanup errors:", cleanup_errors(stale_cleanup)),
        *mirror_sync_error_lines("Discord orphan cleanup errors:", cleanup_errors(orphan_cleanup)),
        *mirror_sync_error_lines(
            "Discord stale project cleanup errors:",
            cleanup_errors(stale_project_cleanup),
        ),
    ]
    return "\n".join(lines)
