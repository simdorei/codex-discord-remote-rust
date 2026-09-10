from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from collections.abc import Awaitable, Callable
from contextlib import AbstractAsyncContextManager
from dataclasses import dataclass
from typing import Protocol, TypeAlias

ApprovalFollowupLoop: TypeAlias = asyncio.AbstractEventLoop
LogLengthValue: TypeAlias = int | str


class ApprovalFollowupChannel(Protocol):
    pass


class ApprovalFollowupWatchResult(Protocol):
    @property
    def session_path(self) -> str | None:
        ...

    @property
    def start_offset(self) -> int | None:
        ...

    @property
    def target_thread_id(self) -> str | None:
        ...

    @property
    def target_ref(self) -> str | None:
        ...


class ApprovalFollowupRelay(Protocol):
    sent_live: bool
    saw_final: bool
    saw_aborted: bool
    saw_timeout: bool


class MakeRelayFunc(Protocol):
    def __call__(
        self,
        loop: ApprovalFollowupLoop,
        channel: ApprovalFollowupChannel,
        target_thread_id: str,
        target_ref: str,
    ) -> ApprovalFollowupRelay:
        ...


class ChannelTypingFunc(Protocol):
    def __call__(
        self,
        channel: ApprovalFollowupChannel,
        *,
        context: str,
    ) -> AbstractAsyncContextManager[None]:
        ...


class WatchStreamFunc(Protocol):
    def __call__(
        self,
        watch_result: ApprovalFollowupWatchResult,
        relay: ApprovalFollowupRelay,
        *,
        timeout_sec: float,
    ) -> tuple[int, str]:
        ...


class SendChunksFunc(Protocol):
    def __call__(self, channel: ApprovalFollowupChannel, content: str) -> Awaitable[int | None]:
        ...


@dataclass(frozen=True, slots=True)
class ApprovalFollowupDeps:
    make_relay: MakeRelayFunc
    get_watch_timeout: Callable[[], float]
    channel_typing: ChannelTypingFunc
    run_watch_stream: WatchStreamFunc
    send_chunks: SendChunksFunc
    log_line: Callable[[str], None]
    format_log_text_len: Callable[[str], LogLengthValue]


async def stream_post_approval_result_to_channel(
    channel: ApprovalFollowupChannel,
    watch_result: ApprovalFollowupWatchResult | None,
    target_thread_id: str,
    *,
    deps: ApprovalFollowupDeps,
) -> bool:
    if watch_result is None:
        return False
    if not watch_result.session_path or watch_result.start_offset is None:
        deps.log_line(f"approval_followup_watch_unavailable target={target_thread_id} reason=no_session")
        return False

    relay = deps.make_relay(
        asyncio.get_running_loop(),
        channel,
        watch_result.target_thread_id or target_thread_id,
        watch_result.target_ref or target_thread_id,
    )
    timeout_sec = deps.get_watch_timeout()
    async with deps.channel_typing(channel, context="approval_followup_watch"):
        exit_code, output = await asyncio.to_thread(
            deps.run_watch_stream,
            watch_result,
            relay,
            timeout_sec=timeout_sec,
        )
    deps.log_line(
        f"approval_followup_watch_done exit={exit_code} target={target_thread_id} "
        + f"sent_live={relay.sent_live} final={relay.saw_final} aborted={relay.saw_aborted} "
        + f"timeout={relay.saw_timeout} output_len={deps.format_log_text_len(output)}"
    )
    if relay.sent_live:
        if exit_code == 0 and not relay.saw_aborted:
            if relay.saw_final:
                return True
            deps.log_line(
                f"approval_followup_watch_no_final_fallback target={target_thread_id} "
                + f"output_len={deps.format_log_text_len(output)}"
            )
            await deps.send_chunks(
                channel,
                f"Approval follow-up finished\n\n{output or '(no final answer captured)'}",
            )
        elif not relay.saw_aborted and not relay.saw_timeout:
            await deps.send_chunks(
                channel,
                f"Approval follow-up watch failed (exit {exit_code})\n\n{output or '(no output)'}",
            )
        return True
    if exit_code != 0 and relay.saw_timeout:
        deps.log_line(
            f"approval_followup_watch_timeout_suppressed target={target_thread_id} "
            + f"exit={exit_code} output_len={deps.format_log_text_len(output)}"
        )
        return True
    if exit_code != 0 and not output:
        deps.log_line(f"approval_followup_watch_empty_failure_suppressed target={target_thread_id} exit={exit_code}")
        return True
    if exit_code == 0 and not output:
        deps.log_line(f"approval_followup_watch_empty_success_suppressed target={target_thread_id}")
        return True

    title = "Approval follow-up finished" if exit_code == 0 else f"Approval follow-up watch failed (exit {exit_code})"
    await deps.send_chunks(channel, f"{title}\n\n{output or '(no output)'}")
    return True
