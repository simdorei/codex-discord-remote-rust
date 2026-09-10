from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Generic, Protocol, TypeVar


ChannelContraT = TypeVar("ChannelContraT", contravariant=True)
ChannelT = TypeVar("ChannelT")
LogFunc = Callable[[str], None]
TextLenFunc = Callable[[str | None], int]
SteeringHandoffPredicate = Callable[[str | None, float], bool]


class AskStreamRelay(Protocol):
    @property
    def sent_live(self) -> bool: ...

    @property
    def saw_final(self) -> bool: ...

    @property
    def saw_aborted(self) -> bool: ...

    @property
    def saw_timeout(self) -> bool: ...


class ChunkSender(Protocol[ChannelContraT]):
    def __call__(
        self,
        channel: ChannelContraT,
        content: str,
        *,
        context: str | None = None,
    ) -> Awaitable[None]: ...


@dataclass(frozen=True, slots=True)
class AskStreamResultDeps(Generic[ChannelT]):
    send_chunks: ChunkSender[ChannelT]
    had_steering_handoff_since: SteeringHandoffPredicate
    format_log_text_len: TextLenFunc
    log: LogFunc


def make_ask_stream_result_deps(
    *,
    send_chunks: ChunkSender[ChannelT],
    had_steering_handoff_since: SteeringHandoffPredicate,
    format_log_text_len: TextLenFunc,
    log: LogFunc,
) -> AskStreamResultDeps[ChannelT]:
    return AskStreamResultDeps(
        send_chunks=send_chunks,
        had_steering_handoff_since=had_steering_handoff_since,
        format_log_text_len=format_log_text_len,
        log=log,
    )


async def handle_ask_stream_result(
    channel: ChannelT,
    *,
    exit_code: int,
    output: str,
    relay: AskStreamRelay,
    target_thread_id: str | None,
    started_at: float,
    delegate_to_session_mirror: bool,
    deps: AskStreamResultDeps[ChannelT],
) -> None:
    if delegate_to_session_mirror and exit_code == 0 and not relay.saw_aborted and not relay.saw_timeout:
        deps.log(
            f"ask_stream_delegated_to_session_mirror target={target_thread_id or '-'} "
            + f"sent_live={relay.sent_live} final={relay.saw_final} output_len={deps.format_log_text_len(output)}"
        )
        return
    if (
        exit_code == 0
        and not relay.saw_final
        and not relay.saw_aborted
        and not relay.saw_timeout
        and deps.had_steering_handoff_since(target_thread_id, started_at)
    ):
        deps.log(
            f"ask_stream_suppressed_after_steering target={target_thread_id or '-'} "
            + f"sent_live={relay.sent_live} output_len={deps.format_log_text_len(output)}"
        )
        return
    if relay.sent_live:
        await _handle_sent_live_result(channel, exit_code=exit_code, output=output, relay=relay, target_thread_id=target_thread_id, deps=deps)
        return
    await deps.send_chunks(channel, _build_terminal_message(exit_code, output, relay, target_thread_id, deps))


async def _handle_sent_live_result(
    channel: ChannelT,
    *,
    exit_code: int,
    output: str,
    relay: AskStreamRelay,
    target_thread_id: str | None,
    deps: AskStreamResultDeps[ChannelT],
) -> None:
    if exit_code == 0 and not relay.saw_aborted:
        if relay.saw_final:
            return
        deps.log(
            f"ask_stream_no_final_error target={target_thread_id or '-'} "
            + f"output_len={deps.format_log_text_len(output)}"
        )
        await deps.send_chunks(channel, "Ask failed\n\n" + _no_final_output(output))
    elif not relay.saw_aborted and not relay.saw_timeout:
        await deps.send_chunks(channel, f"Ask failed (exit {exit_code})\n\n{output or '(no output)'}")


def _build_terminal_message(
    exit_code: int,
    output: str,
    relay: AskStreamRelay,
    target_thread_id: str | None,
    deps: AskStreamResultDeps[ChannelT],
) -> str:
    if exit_code == 0 and not relay.saw_final and not relay.saw_aborted and not relay.saw_timeout:
        deps.log(
            f"ask_stream_no_final_error target={target_thread_id or '-'} "
            + f"output_len={deps.format_log_text_len(output)}"
        )
        return "Ask failed\n\n" + _no_final_output(output)
    title = "Ask finished" if exit_code == 0 else f"Ask failed (exit {exit_code})"
    return f"{title}\n\n{output or '(no output)'}"


def _no_final_output(output: str) -> str:
    return "ERROR: Codex stream completed without a final answer.\n\n" + (output or "(no output)")
