from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Protocol, TypeAlias, TypeVar


class ThreadLike(Protocol):
    @property
    def rollout_path(self) -> str: ...


PendingStateT = TypeVar("PendingStateT")
ThreadT = TypeVar("ThreadT", bound=ThreadLike)
LogLengthValue: TypeAlias = int | str


class StaleBusySteerChannel(Protocol):
    pass


class BuildStaleBusySteerBlockMessageFunc(Protocol):
    def __call__(self, prompt: str, *, target_ref: str, age_seconds: float) -> str:
        ...


class SendChunksFunc(Protocol):
    def __call__(self, channel: StaleBusySteerChannel, content: str) -> Awaitable[int | None]:
        ...


@dataclass(frozen=True, slots=True)
class StaleBusySteerBlockDeps:
    get_block_info: Callable[[str | None], tuple[str, str, float] | None]
    build_message: BuildStaleBusySteerBlockMessageFunc
    send_chunks: SendChunksFunc
    log: Callable[[str], None]
    format_log_text_len: Callable[[str], LogLengthValue]


def get_stale_busy_steer_block_info(
    target_thread_id: str | None,
    *,
    resolve_target_ref: Callable[[str | None], tuple[str | None, str]],
    choose_thread: Callable[[str], ThreadT],
    is_thread_busy: Callable[[Path], bool],
    get_pending_interactive_state: Callable[[Path], PendingStateT | None],
    session_file_age_seconds: Callable[[Path], float | None],
    get_thread_workspace_ref: Callable[[ThreadT], str],
    stale_seconds: float,
) -> tuple[str, str, float] | None:
    resolved_thread_id, target_ref = resolve_target_ref(target_thread_id)
    if not resolved_thread_id:
        return None
    thread = choose_thread(resolved_thread_id)
    session_path = Path(thread.rollout_path)
    if not session_path.exists() or not is_thread_busy(session_path):
        return None
    if get_pending_interactive_state(session_path):
        return None
    age_seconds = session_file_age_seconds(session_path)
    if age_seconds is None or age_seconds < stale_seconds:
        return None
    return resolved_thread_id, target_ref or get_thread_workspace_ref(thread), age_seconds


async def send_stale_busy_steer_block_message(
    channel: StaleBusySteerChannel,
    prompt: str,
    target_thread_id: str | None,
    *,
    reason: str,
    deps: StaleBusySteerBlockDeps,
) -> bool:
    info = deps.get_block_info(target_thread_id)
    if info is None:
        return False
    resolved_thread_id, target_ref, age_seconds = info
    await deps.send_chunks(
        channel,
        deps.build_message(prompt, target_ref=target_ref, age_seconds=age_seconds),
    )
    deps.log(
        f"stale_busy_steer_blocked reason={reason} target={resolved_thread_id} "
        + f"age_sec={age_seconds:.1f} prompt_len={deps.format_log_text_len(prompt)}"
    )
    return True
