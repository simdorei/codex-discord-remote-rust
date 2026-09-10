from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Generic, TypeVar

import codex_discord_session_mirror_archive as session_mirror_archive

ThreadT = TypeVar("ThreadT")
ContextUsageT = TypeVar("ContextUsageT")


@dataclass(frozen=True, slots=True)
class SessionMirrorThreadPolicy(Generic[ThreadT]):
    codex_thread: ThreadT
    active_output_target: bool
    archive_tail_only: bool


@dataclass(frozen=True, slots=True)
class SessionMirrorEventPolicyDeps(Generic[ThreadT, ContextUsageT]):
    choose_thread: Callable[[str], Awaitable[ThreadT]]
    get_thread_context_usage: Callable[[ThreadT], Awaitable[ContextUsageT]]
    should_recommend_archive: Callable[[ThreadT, ContextUsageT], bool]
    is_active_output_target: Callable[[str], bool]
    archive_skip_logged: set[str]
    log: session_mirror_archive.LogFunc


async def prepare_session_mirror_thread_policy(
    codex_thread_id: str,
    *,
    deps: SessionMirrorEventPolicyDeps[ThreadT, ContextUsageT],
) -> SessionMirrorThreadPolicy[ThreadT] | None:
    try:
        codex_thread = await deps.choose_thread(codex_thread_id)
    except Exception as exc:  # noqa: BROAD_EXCEPT_OK - preserves existing mirror boundary logging.
        deps.log(
            f"session_mirror_thread_unavailable target={codex_thread_id} "
            + f"error_type={type(exc).__name__}"
        )
        return None

    active_output_target = deps.is_active_output_target(codex_thread_id)
    if codex_thread_id in deps.archive_skip_logged and not active_output_target:
        return SessionMirrorThreadPolicy(
            codex_thread=codex_thread,
            active_output_target=active_output_target,
            archive_tail_only=True,
        )
    try:
        context_usage = await deps.get_thread_context_usage(codex_thread)
    except MemoryError as exc:
        deps.log(
            f"session_mirror_context_usage_failed target={codex_thread_id} "
            + f"error_type={type(exc).__name__} action=archive_tail_only"
        )
        archive_recommended = True
    else:
        archive_recommended = deps.should_recommend_archive(codex_thread, context_usage)
    archive_tail_only = session_mirror_archive.resolve_session_mirror_archive_policy(
        codex_thread_id,
        archive_recommended=archive_recommended,
        active_output_target=active_output_target,
        archive_skip_logged=deps.archive_skip_logged,
        log=deps.log,
    )
    return SessionMirrorThreadPolicy(
        codex_thread=codex_thread,
        active_output_target=active_output_target,
        archive_tail_only=archive_tail_only,
    )
