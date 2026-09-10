from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Generic, Protocol, TypeVar

from codex_thread_models import ThreadInfo


ChannelContraT = TypeVar("ChannelContraT", contravariant=True)
ChannelT = TypeVar("ChannelT")
RecentOffsets = dict[str, tuple[ThreadInfo, Path, int]]
LogFunc = Callable[[str], None]
TextLenFunc = Callable[[str | None], int]
SteeringHandoffMarker = Callable[[str], None]


class RecordedBusyHandler(Protocol[ChannelContraT]):
    def __call__(
        self,
        channel: ChannelContraT,
        prompt: str,
        *,
        target_thread_id: str | None,
        target_ref: str,
        recent_offsets: RecentOffsets,
        transport_output: str,
        delegate_to_session_mirror: bool,
    ) -> Awaitable[bool]: ...


class BusySettleWaiter(Protocol):
    def __call__(
        self,
        prompt: str,
        *,
        target_thread_id: str | None,
        recent_offsets: RecentOffsets,
    ) -> Awaitable[None]: ...


class AppMenuSender(Protocol[ChannelContraT]):
    def __call__(
        self,
        channel: ChannelContraT,
        target_thread_id: str | None,
        output: str,
        *,
        reason: str,
    ) -> Awaitable[bool]: ...


@dataclass(frozen=True, slots=True)
class AskStreamBusyResultDeps(Generic[ChannelT]):
    handle_recorded_busy_transport_prompt: RecordedBusyHandler[ChannelT]
    wait_for_mirrored_busy_delegation_settle: BusySettleWaiter
    mark_steering_handoff: SteeringHandoffMarker
    send_codex_app_menu_if_available: AppMenuSender[ChannelT]
    format_log_text_len: TextLenFunc
    log: LogFunc


async def handle_ask_stream_busy_result(
    channel: ChannelT,
    prompt: str,
    *,
    target_thread_id: str | None,
    target_ref: str,
    recent_offsets: RecentOffsets,
    transport_output: str,
    delegate_to_session_mirror: bool,
    retry_index: int | None,
    deps: AskStreamBusyResultDeps[ChannelT],
) -> bool:
    if await deps.handle_recorded_busy_transport_prompt(
        channel,
        prompt,
        target_thread_id=target_thread_id,
        target_ref=target_ref,
        recent_offsets=recent_offsets,
        transport_output=transport_output,
        delegate_to_session_mirror=delegate_to_session_mirror,
    ):
        return True
    if delegate_to_session_mirror:
        _log_delegated_busy(retry_index, target_thread_id, transport_output, deps)
        if target_thread_id:
            deps.mark_steering_handoff(target_thread_id)
        await deps.wait_for_mirrored_busy_delegation_settle(
            prompt,
            target_thread_id=target_thread_id,
            recent_offsets=recent_offsets,
        )
        return True
    reason = "ask_target_busy_failure" if retry_index is None else f"ask_busy_retry_{retry_index}"
    return await deps.send_codex_app_menu_if_available(
        channel,
        target_thread_id,
        transport_output,
        reason=reason,
    )


def _log_delegated_busy(
    retry_index: int | None,
    target_thread_id: str | None,
    transport_output: str,
    deps: AskStreamBusyResultDeps[ChannelT],
) -> None:
    if retry_index is None:
        deps.log(
            f"ask_stream_busy_delegated_to_session_mirror target={target_thread_id or '-'} "
            + f"output_len={deps.format_log_text_len(transport_output)}"
        )
        return
    deps.log(
        f"ask_stream_retry_busy_delegated_to_session_mirror attempt={retry_index} "
        + f"target={target_thread_id or '-'} output_len={deps.format_log_text_len(transport_output)}"
    )


def build_codex_app_busy_retry_message(target_ref: str, attempts: int) -> str:
    lines = [
        "Codex app did not accept this Discord message yet.",
        "",
        f"target: `{target_ref or 'selected'}`",
        f"retry_attempts: {attempts}",
        "",
        "No approval/input menu was exposed by the Codex app for this turn.",
        "The Discord message stayed in this thread; no steering menu was created without a Codex app menu to mirror.",
    ]
    return "\n".join(lines)


def build_codex_app_steering_not_accepted_message(target_ref: str) -> str:
    lines = [
        "Codex app did not accept this steering message yet.",
        "",
        f"target: `{target_ref or 'selected'}`",
        "",
        "No approval/input menu was exposed by the Codex app for this turn.",
        "The original Discord controls were cleared; send a new message in the mapped thread after Codex output or an app menu appears.",
    ]
    return "\n".join(lines)
