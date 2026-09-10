from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from collections.abc import Awaitable, Callable
from contextlib import AbstractAsyncContextManager
from dataclasses import dataclass
from typing import Protocol, TypeAlias

SteeringWatchLoop: TypeAlias = asyncio.AbstractEventLoop
LogLengthValue: TypeAlias = int | str


class SteeringWatchChannel(Protocol):
    pass


class SteeringWatchResult(Protocol):
    @property
    def session_path(self) -> str | None: ...

    @property
    def start_offset(self) -> int | None: ...

    @property
    def target_thread_id(self) -> str | None: ...

    @property
    def target_ref(self) -> str | None: ...

    @property
    def delivery_pending(self) -> bool: ...


class SteeringWatchRelay(Protocol):
    @property
    def sent_live(self) -> bool: ...

    @property
    def saw_final(self) -> bool: ...

    @property
    def saw_aborted(self) -> bool: ...

    @property
    def saw_timeout(self) -> bool: ...

    @property
    def suppressed_after_steering(self) -> bool: ...


class MakeRelayFunc(Protocol):
    def __call__(
        self,
        loop: SteeringWatchLoop,
        channel: SteeringWatchChannel,
        target_thread_id: str,
        target_ref: str,
        *,
        started_at: float,
        send_commentary_blocks: bool | None,
        send_final_blocks: bool,
    ) -> SteeringWatchRelay: ...


class ChannelTypingFunc(Protocol):
    def __call__(
        self,
        channel: SteeringWatchChannel,
        *,
        context: str,
    ) -> AbstractAsyncContextManager[None]: ...


class WatchStreamFunc(Protocol):
    def __call__(
        self,
        watch_result: SteeringWatchResult,
        relay: SteeringWatchRelay,
        *,
        timeout_sec: float,
    ) -> tuple[int, str]: ...


class SendChunksFunc(Protocol):
    def __call__(self, channel: SteeringWatchChannel, content: str) -> Awaitable[int | None]: ...


@dataclass(frozen=True, slots=True)
class SteeringWatchDeps:
    monotonic: Callable[[], float]
    make_relay: MakeRelayFunc
    get_watch_timeout: Callable[[], float]
    channel_typing: ChannelTypingFunc
    run_watch_stream: WatchStreamFunc
    send_chunks: SendChunksFunc
    log_line: Callable[[str], None]
    format_log_text_len: Callable[[str], LogLengthValue]


async def stream_steering_prompt_result_to_channel(
    channel: SteeringWatchChannel,
    steering_result: SteeringWatchResult | None,
    target_thread_id: str | None,
    *,
    label: str = "Steering",
    send_commentary_blocks: bool | None = None,
    send_final_blocks: bool = True,
    deps: SteeringWatchDeps,
) -> bool:
    if steering_result is None:
        return False
    if not steering_result.session_path or steering_result.start_offset is None:
        deps.log_line(f"steer_watch_unavailable target={target_thread_id or '-'}")
        return False
    started_at = deps.monotonic()
    relay = deps.make_relay(
        asyncio.get_running_loop(),
        channel,
        steering_result.target_thread_id or target_thread_id or "-",
        steering_result.target_ref or target_thread_id or "-",
        started_at=started_at,
        send_commentary_blocks=send_commentary_blocks,
        send_final_blocks=send_final_blocks,
    )
    timeout_sec = deps.get_watch_timeout()
    async with deps.channel_typing(channel, context="steer_watch"):
        exit_code, output = await asyncio.to_thread(
            deps.run_watch_stream,
            steering_result,
            relay,
            timeout_sec=timeout_sec,
        )
    deps.log_line(
        f"steer_watch_done exit={exit_code} target={target_thread_id or '-'} "
        f"sent_live={relay.sent_live} final={relay.saw_final} aborted={relay.saw_aborted} "
        f"timeout={relay.saw_timeout} suppressed={relay.suppressed_after_steering} "
        f"pending={steering_result.delivery_pending} output_len={deps.format_log_text_len(output)}"
    )
    if relay.suppressed_after_steering:
        deps.log_line(f"steer_watch_suppressed_after_newer_handoff target={target_thread_id or '-'}")
        return True
    if send_commentary_blocks is False and not send_final_blocks and not (exit_code != 0 and relay.saw_timeout):
        deps.log_line(
            f"steer_watch_public_output_delegated_to_session_mirror target={target_thread_id or '-'} "
            f"exit={exit_code} output_len={deps.format_log_text_len(output)}"
        )
        return True
    if relay.sent_live:
        if exit_code == 0 and not relay.saw_aborted:
            if relay.saw_final:
                return True
            deps.log_line(
                f"steer_watch_no_final_fallback target={target_thread_id or '-'} "
                f"output_len={deps.format_log_text_len(output)}"
            )
            await deps.send_chunks(channel, f"{label} finished\n\n{output or '(no final answer captured)'}")
        elif not relay.saw_aborted and not relay.saw_timeout:
            await deps.send_chunks(channel, f"{label} watch failed (exit {exit_code})\n\n{output or '(no output)'}")
        return True
    if exit_code == 0 and relay.saw_final and not relay.sent_live:
        safe_label = label.replace(chr(10), " ")[:40]
        deps.log_line(
            f"steer_watch_final_suppressed target={target_thread_id or '-'} "
            f"label={safe_label} output_len={deps.format_log_text_len(output)}"
        )
        return True
    if exit_code != 0 and relay.saw_timeout:
        deps.log_line(
            f"steer_watch_timeout_reported target={target_thread_id or '-'} "
            f"exit={exit_code} pending={steering_result.delivery_pending} "
            f"output_len={deps.format_log_text_len(output)}"
        )
        await deps.send_chunks(
            channel,
            "\n".join(
                [
                    f"{label} is still running in Codex.",
                    "",
                    "No final answer was captured before the Discord watch timeout. Do not resend the same message yet; check the Codex thread or wait for the next relay.",
                ]
            ),
        )
        return True
    if exit_code != 0 and not output:
        deps.log_line(
            f"steer_watch_empty_failure_suppressed target={target_thread_id or '-'} "
            f"exit={exit_code} pending={steering_result.delivery_pending}"
        )
        return True
    if exit_code == 0 and not output:
        deps.log_line(
            f"steer_watch_empty_success_suppressed target={target_thread_id or '-'} "
            f"pending={steering_result.delivery_pending}"
        )
        return True
    title = f"{label} finished" if exit_code == 0 else f"{label} watch failed (exit {exit_code})"
    await deps.send_chunks(channel, f"{title}\n\n{output or '(no output)'}")
    return True
