from __future__ import annotations

from collections.abc import Awaitable, Callable
from contextlib import AbstractAsyncContextManager
from dataclasses import dataclass
from typing import Generic, Protocol, TypeVar


ChannelContraT = TypeVar("ChannelContraT", contravariant=True)
ChannelT = TypeVar("ChannelT")
RelayT = TypeVar("RelayT", bound="RetryRelay")
RelayT_co = TypeVar("RelayT_co", bound="RetryRelay", covariant=True)
RelayT_contra = TypeVar("RelayT_contra", bound="RetryRelay", contravariant=True)
LogFunc = Callable[[str], None]
TextLenFunc = Callable[[str | None], int]
AsyncSleeper = Callable[[float], Awaitable[None]]


class RetryRelay(Protocol):
    @property
    def sent_live(self) -> bool: ...

    @property
    def saw_final(self) -> bool: ...

    @property
    def saw_aborted(self) -> bool: ...

    @property
    def saw_timeout(self) -> bool: ...


class RetryRelayFactory(Protocol[ChannelContraT, RelayT_co]):
    def __call__(
        self,
        channel: ChannelContraT,
        *,
        target_thread_id: str | None,
        target_ref: str,
        started_at: float,
        delegate_to_session_mirror: bool,
    ) -> RelayT_co: ...


class ChannelTypingFactory(Protocol[ChannelContraT]):
    def __call__(self, channel: ChannelContraT, *, context: str) -> AbstractAsyncContextManager[None]: ...


class AskStreamRunner(Protocol[RelayT_contra]):
    def __call__(
        self,
        prompt: str,
        relay: RelayT_contra,
        *,
        target_thread_id: str | None,
    ) -> Awaitable[tuple[int, str]]: ...


@dataclass(frozen=True, slots=True)
class RetryAttemptResult(Generic[RelayT]):
    exit_code: int
    output: str
    relay: RelayT


@dataclass(frozen=True, slots=True)
class RetryAttemptDeps(Generic[ChannelT, RelayT]):
    sleep: AsyncSleeper
    make_retry_relay: RetryRelayFactory[ChannelT, RelayT]
    channel_typing: ChannelTypingFactory[ChannelT]
    run_ask_stream: AskStreamRunner[RelayT]
    format_log_text_len: TextLenFunc
    log: LogFunc


async def run_retry_attempt(
    channel: ChannelT,
    *,
    prompt: str,
    retry_index: int,
    retry_delay: float,
    target_thread_id: str | None,
    target_ref: str,
    started_at: float,
    delegate_to_session_mirror: bool,
    deps: RetryAttemptDeps[ChannelT, RelayT],
) -> RetryAttemptResult[RelayT]:
    await deps.sleep(retry_delay)
    relay = deps.make_retry_relay(
        channel,
        target_thread_id=target_thread_id,
        target_ref=target_ref,
        started_at=started_at,
        delegate_to_session_mirror=delegate_to_session_mirror,
    )
    async with deps.channel_typing(channel, context="ask_stream_retry"):
        exit_code, output = await deps.run_ask_stream(prompt, relay, target_thread_id=target_thread_id)
    _log_retry_done(
        retry_index=retry_index,
        exit_code=exit_code,
        output=output,
        relay=relay,
        target_thread_id=target_thread_id,
        deps=deps,
    )
    return RetryAttemptResult(exit_code=exit_code, output=output, relay=relay)


def _log_retry_done(
    *,
    retry_index: int,
    exit_code: int,
    output: str,
    relay: RetryRelay,
    target_thread_id: str | None,
    deps: RetryAttemptDeps[ChannelT, RelayT],
) -> None:
    deps.log(
        f"ask_stream_retry_done attempt={retry_index} exit={exit_code} target={target_thread_id or '-'} "
        + f"sent_live={relay.sent_live} final={relay.saw_final} aborted={relay.saw_aborted} "
        + f"timeout={relay.saw_timeout} output_len={deps.format_log_text_len(output)}"
    )
