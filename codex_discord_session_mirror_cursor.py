from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Protocol

LogFunc = Callable[[str], None]
PendingCursorPredicate = Callable[[str], bool]
PendingCursorClearer = Callable[[str], None]
MirrorCursorExceptionTypes = tuple[type[BaseException], ...]


class SessionMirrorCursorThread(Protocol):
    @property
    def rollout_path(self) -> str: ...


class SessionMirrorCursorUpdater(Protocol):
    def __call__(self, codex_thread_id: str, rollout_path: str, cursor: int) -> Awaitable[None]: ...


class SessionMirrorCursorGetter(Protocol):
    def __call__(self, codex_thread_id: str, rollout_path: str, initial_cursor: int) -> Awaitable[int]: ...


class SessionMirrorCursorChooser(Protocol):
    def __call__(self, ref: str, fallback: str | None = None) -> SessionMirrorCursorThread: ...


@dataclass(frozen=True, slots=True)
class PrimeSessionMirrorCursorDeps:
    session_mirror_enabled: Callable[[], bool]
    choose_thread: SessionMirrorCursorChooser
    get_session_mirror_offset: Callable[[str], tuple[str, int, float] | None]
    get_or_init_session_mirror_cursor: Callable[[str, str, int], int]
    update_session_mirror_cursor: Callable[[str, str, int], None]
    is_active_output_target: Callable[[str], bool]
    preserve_seconds: float
    time_now: Callable[[], float]
    exception_types: MirrorCursorExceptionTypes
    format_exception: Callable[[], str]
    log: LogFunc


@dataclass(frozen=True, slots=True)
class SessionMirrorCursorInitDeps:
    is_pending_cursor_target: PendingCursorPredicate
    clear_pending_cursor_target: PendingCursorClearer
    update_session_mirror_cursor: SessionMirrorCursorUpdater
    get_or_init_session_mirror_cursor: SessionMirrorCursorGetter
    log: LogFunc


async def initialize_session_mirror_cursor(
    codex_thread_id: str,
    rollout_path: str,
    *,
    session_size: int,
    active_output_target: bool,
    deps: SessionMirrorCursorInitDeps,
) -> int:
    pending_cursor = deps.is_pending_cursor_target(codex_thread_id)
    initial_cursor = 0 if active_output_target and pending_cursor else session_size
    if active_output_target and pending_cursor:
        await deps.update_session_mirror_cursor(codex_thread_id, rollout_path, initial_cursor)
        cursor = initial_cursor
    else:
        cursor = await deps.get_or_init_session_mirror_cursor(codex_thread_id, rollout_path, initial_cursor)
    if pending_cursor:
        deps.clear_pending_cursor_target(codex_thread_id)
        deps.log(f"session_mirror_pending_cursor_initialized target={codex_thread_id} cursor={initial_cursor}")
    return cursor


def prime_session_mirror_cursor_for_target(
    target_thread_id: str | None,
    *,
    deps: PrimeSessionMirrorCursorDeps,
) -> int | None:
    if not deps.session_mirror_enabled() or not target_thread_id:
        return None
    try:
        codex_thread = deps.choose_thread(target_thread_id, None)
        session_path = Path(codex_thread.rollout_path)
        if not session_path.exists():
            return None
        rollout_path = str(session_path)
        current_cursor = session_path.stat().st_size
        offset = deps.get_session_mirror_offset(target_thread_id)
        offset_updated_at = 0.0
        if offset and offset[0] == rollout_path:
            offset_updated_at = offset[2]
        cursor = deps.get_or_init_session_mirror_cursor(
            target_thread_id,
            rollout_path,
            current_cursor,
        )
        if cursor != current_cursor:
            preserve_reason = ""
            if cursor < current_cursor and deps.is_active_output_target(target_thread_id):
                preserve_reason = "active_output"
            elif (
                cursor < current_cursor
                and offset_updated_at
                and deps.time_now() - offset_updated_at <= deps.preserve_seconds
            ):
                preserve_reason = "recent_cursor"
            if preserve_reason:
                deps.log(
                    f"session_mirror_cursor_prime_preserved target={target_thread_id} "
                    + f"cursor={cursor} current={current_cursor} reason={preserve_reason}"
                )
            else:
                deps.update_session_mirror_cursor(target_thread_id, rollout_path, current_cursor)
                cursor = current_cursor
        deps.log(f"session_mirror_cursor_primed target={target_thread_id} cursor={cursor}")
        return cursor
    except deps.exception_types:
        deps.log(
            f"session_mirror_cursor_prime_failed target={target_thread_id or '-'}\n"
            + deps.format_exception()
        )
        return None
