from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from collections.abc import Awaitable, Callable
from typing import Protocol, TypeAlias, TypeVar

from codex_thread_models import ThreadInfo

ExceptionTypes: TypeAlias = tuple[type[BaseException], ...]

ChannelT = TypeVar("ChannelT")
ChannelT_contra = TypeVar("ChannelT_contra", contravariant=True)
SendResultT = TypeVar("SendResultT")
SendResultT_co = TypeVar("SendResultT_co", covariant=True)


class SessionMirrorOutputChannel(Protocol):
    @property
    def id(self) -> int | None: ...


class ThreadContextUsageLike(Protocol):
    model_context_window: int
    peak_input_tokens: int
    last_total_tokens: int


class MirrorStatusBridge(Protocol):
    def choose_thread(self, thread_id: str, cwd: str | None) -> ThreadInfo: ...

    def get_thread_context_usage(self, thread: ThreadInfo) -> ThreadContextUsageLike | None: ...

    def should_recommend_archive(
        self,
        thread: ThreadInfo,
        context_usage: ThreadContextUsageLike | None,
    ) -> bool: ...


class ContextFormatBridge(Protocol):
    def format_token_k(self, tokens: int) -> str: ...


class SendChunksFunc(Protocol[ChannelT_contra, SendResultT_co]):
    def __call__(
        self,
        target: ChannelT_contra,
        text: str,
        *,
        context: str,
    ) -> Awaitable[SendResultT_co]: ...


class ShouldDelegateFunc(Protocol[ChannelT_contra]):
    def __call__(self, channel: ChannelT_contra, target_thread_id: str | None) -> bool: ...


class PrimeCursorFunc(Protocol):
    def __call__(self, target_thread_id: str | None) -> int | None: ...


def should_delegate_output_to_session_mirror(
    channel: SessionMirrorOutputChannel,
    target_thread_id: str | None,
    *,
    session_mirror_enabled_func: Callable[[], bool],
    get_mirrored_codex_thread_id_func: Callable[[int | None], str | None],
    bridge_module: MirrorStatusBridge,
    expected_exceptions: ExceptionTypes,
    log_func: Callable[[str], None],
) -> bool:
    if not session_mirror_enabled_func() or not target_thread_id:
        return False
    channel_id = channel.id
    try:
        if get_mirrored_codex_thread_id_func(channel_id) != target_thread_id:
            return False
    except expected_exceptions as exc:
        log_func(
            f"session_mirror_delegate_disabled target={target_thread_id} "
            + f"reason=mapping_unavailable channel={channel_id or '-'} error_type={type(exc).__name__}"
        )
        return False
    try:
        codex_thread = bridge_module.choose_thread(target_thread_id, None)
        context_usage = bridge_module.get_thread_context_usage(codex_thread)
    except expected_exceptions as exc:
        log_func(
            f"session_mirror_delegate_disabled target={target_thread_id} "
            + f"reason=thread_unavailable error_type={type(exc).__name__}"
        )
        return False
    if bridge_module.should_recommend_archive(codex_thread, context_usage):
        log_func(f"session_mirror_delegate_disabled target={target_thread_id} reason=archive_recommended")
        return False
    return True


def is_context_exhausted_no_reply_state(context_usage: ThreadContextUsageLike | None) -> bool:
    if context_usage is None:
        return False
    model_context_window = int(getattr(context_usage, "model_context_window", 0) or 0)
    if model_context_window <= 0:
        return False
    peak_input_tokens = int(getattr(context_usage, "peak_input_tokens", 0) or 0)
    last_total_tokens = int(getattr(context_usage, "last_total_tokens", 0) or 0)
    return last_total_tokens == 0 and peak_input_tokens >= int(model_context_window * 0.90)


def build_context_exhausted_prompt_message(
    target_ref: str,
    context_usage: ThreadContextUsageLike,
    *,
    format_token_k_func: Callable[[int], str],
) -> str:
    peak_input = format_token_k_func(int(getattr(context_usage, "peak_input_tokens", 0) or 0))
    window = format_token_k_func(int(getattr(context_usage, "model_context_window", 0) or 0))
    return (
        f"Codex thread `{target_ref}` is in a no-visible-reply state.\n"
        f"The last turn recorded no assistant reply after a high context peak ({peak_input}/{window}).\n"
        f"Run `!archive {target_ref}`, then `!mirror sync`, then resend."
    )


async def send_context_exhausted_prompt_notice_if_needed(
    channel: ChannelT,
    target_thread_id: str | None,
    target_ref: str,
    *,
    bridge_module: MirrorStatusBridge,
    send_chunks_func: SendChunksFunc[ChannelT, SendResultT],
    format_token_k_func: Callable[[int], str],
    expected_exceptions: ExceptionTypes,
    log_func: Callable[[str], None],
) -> bool:
    if not target_thread_id:
        return False
    try:
        codex_thread = bridge_module.choose_thread(target_thread_id, None)
        context_usage = bridge_module.get_thread_context_usage(codex_thread)
    except expected_exceptions as exc:
        log_func(f"ask_context_guard_unavailable target={target_thread_id} error_type={type(exc).__name__}")
        return False
    if context_usage is None:
        return False
    if not is_context_exhausted_no_reply_state(context_usage):
        return False
    log_func(
        f"ask_blocked_context_exhausted target={target_thread_id} "
        + f"peak_input={getattr(context_usage, 'peak_input_tokens', 0) or 0} "
        + f"window={getattr(context_usage, 'model_context_window', 0) or 0}"
    )
    _ = await send_chunks_func(
        channel,
        build_context_exhausted_prompt_message(
            target_ref,
            context_usage,
            format_token_k_func=format_token_k_func,
        ),
        context="ask_context_exhausted",
    )
    return True


async def prepare_session_mirror_delegation(
    channel: ChannelT,
    target_thread_id: str | None,
    *,
    should_delegate_func: ShouldDelegateFunc[ChannelT],
    prime_cursor_func: PrimeCursorFunc,
) -> bool:
    delegate_to_session_mirror = should_delegate_func(channel, target_thread_id)
    if delegate_to_session_mirror:
        _ = await asyncio.to_thread(prime_cursor_func, target_thread_id)
    return delegate_to_session_mirror


__all__ = [
    "ContextFormatBridge",
    "MirrorStatusBridge",
    "PrimeCursorFunc",
    "SendChunksFunc",
    "SessionMirrorOutputChannel",
    "ShouldDelegateFunc",
    "ThreadContextUsageLike",
    "build_context_exhausted_prompt_message",
    "is_context_exhausted_no_reply_state",
    "prepare_session_mirror_delegation",
    "send_context_exhausted_prompt_notice_if_needed",
    "should_delegate_output_to_session_mirror",
]
