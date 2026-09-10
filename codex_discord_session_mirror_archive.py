from __future__ import annotations

from collections.abc import Awaitable, Callable, Mapping
from dataclasses import dataclass
from typing import Protocol, TypedDict, cast

LogFunc = Callable[[str], None]
ExceptionTypes = tuple[type[BaseException], ...]


class ArchiveMirrorCleanupOwner(Protocol):
    _session_mirror_archive_skip_logged: set[str]
    _session_mirror_seen_agent_messages: dict[str, dict[str, float]]
    _session_mirror_seen_user_messages: dict[str, dict[str, float]]


class SessionMirrorStateLike(Protocol):
    active_output_targets: dict[str, float]
    pending_cursor_targets: set[str]


class ArchivedSessionMirrorCleanupCounts(TypedDict):
    mirror_threads: int
    session_mirror_offsets: int
    active_output_targets: int
    pending_cursor_targets: int
    archive_skip_logged: int
    seen_agent_messages: int
    seen_user_messages: int


@dataclass(frozen=True, slots=True)
class ArchiveMirrorCleanupDeps:
    delete_archived_mirror_state: Callable[[str], Mapping[str, int]]
    get_session_mirror_state: Callable[[], SessionMirrorStateLike]
    normalize_runner_key: Callable[[str | None], str]
    release_session_mirror_output_target: Callable[[str | None], Awaitable[bool]]
    parse_bridge_output_value: Callable[[str, str], str]
    format_log_argv: Callable[[list[str]], str]
    exception_types: ExceptionTypes
    format_exception: Callable[[], str]
    log: LogFunc


def resolve_session_mirror_archive_policy(
    codex_thread_id: str,
    *,
    archive_recommended: bool,
    active_output_target: bool,
    archive_skip_logged: set[str],
    log: LogFunc,
) -> bool:
    if archive_recommended and not active_output_target:
        if codex_thread_id not in archive_skip_logged:
            archive_skip_logged.add(codex_thread_id)
            log(f"session_mirror_archive_tail_only target={codex_thread_id} reason=archive_recommended")
        return True
    if active_output_target and codex_thread_id in archive_skip_logged:
        log(f"session_mirror_archive_skip_overridden target={codex_thread_id} reason=active_ask")
    archive_skip_logged.discard(codex_thread_id)
    return False


async def cleanup_archived_session_mirror_state(
    owner: ArchiveMirrorCleanupOwner | None,
    codex_thread_id: str,
    *,
    deps: ArchiveMirrorCleanupDeps,
) -> ArchivedSessionMirrorCleanupCounts:
    state = deps.get_session_mirror_state()
    key = deps.normalize_runner_key(codex_thread_id)
    active_output_targets = int(key in state.active_output_targets)
    pending_cursor_targets = int(key in state.pending_cursor_targets)
    if not await deps.release_session_mirror_output_target(codex_thread_id):
        raise RuntimeError(
            "app-server thread subscription could not be released; "
            + "mirror state was preserved"
        )
    if key in state.active_output_targets:
        raise RuntimeError(
            "mirror output target was reactivated during app-server release; "
            + "mirror state was preserved"
        )
    counts = deps.delete_archived_mirror_state(codex_thread_id)

    archive_skip_logged = 0
    seen_agent_messages = 0
    seen_user_messages = 0
    if owner is not None:
        skip_logged = cast(
            set[str] | None,
            getattr(owner, "_session_mirror_archive_skip_logged", None),
        )
        if skip_logged is not None:
            archive_skip_logged = int(codex_thread_id in skip_logged)
            skip_logged.discard(codex_thread_id)
        seen_agent = cast(
            dict[str, dict[str, float]] | None,
            getattr(owner, "_session_mirror_seen_agent_messages", None),
        )
        if seen_agent is not None:
            seen_agent_messages = int(seen_agent.pop(codex_thread_id, None) is not None)
        seen_user = cast(
            dict[str, dict[str, float]] | None,
            getattr(owner, "_session_mirror_seen_user_messages", None),
        )
        if seen_user is not None:
            seen_user_messages = int(seen_user.pop(codex_thread_id, None) is not None)

    return {
        "mirror_threads": int(counts.get("mirror_threads", 0)),
        "session_mirror_offsets": int(counts.get("session_mirror_offsets", 0)),
        "active_output_targets": active_output_targets,
        "pending_cursor_targets": pending_cursor_targets,
        "archive_skip_logged": archive_skip_logged,
        "seen_agent_messages": seen_agent_messages,
        "seen_user_messages": seen_user_messages,
    }


async def cleanup_archive_mirror_after_bridge_command(
    owner: ArchiveMirrorCleanupOwner | None,
    argv: list[str],
    exit_code: int,
    output: str,
    *,
    deps: ArchiveMirrorCleanupDeps,
) -> str | None:
    if exit_code != 0 or not argv or argv[0] != "archive":
        return None
    archived_thread_id = deps.parse_bridge_output_value(output, "archived_thread")
    if not archived_thread_id:
        deps.log(
            "archive_mirror_cleanup_skipped reason=no_archived_thread "
            + f"argv={deps.format_log_argv(argv)}"
        )
        return None
    try:
        counts = await cleanup_archived_session_mirror_state(
            owner,
            archived_thread_id,
            deps=deps,
        )
    except deps.exception_types as exc:
        deps.log(
            f"archive_mirror_cleanup_failed target={archived_thread_id} "
            + f"error_type={type(exc).__name__}\n"
            + deps.format_exception()
        )
        return f"Mirror cleanup warning: {type(exc).__name__}: {exc}"
    deps.log(
        f"archive_mirror_cleanup_done target={archived_thread_id} "
        + f"mirror_rows={counts['mirror_threads']} "
        + f"offsets={counts['session_mirror_offsets']} "
        + f"active={counts['active_output_targets']} "
        + f"pending={counts['pending_cursor_targets']} "
        + f"seen_agent={counts['seen_agent_messages']} "
        + f"seen_user={counts['seen_user_messages']}"
    )
    return None
