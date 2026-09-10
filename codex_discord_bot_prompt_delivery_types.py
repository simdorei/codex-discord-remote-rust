from __future__ import annotations

from collections.abc import Awaitable, Callable
from contextlib import AbstractAsyncContextManager
from dataclasses import dataclass
from types import TracebackType
from typing import Generic, Protocol, TypeVar

import codex_discord_codex_app_menu as discord_codex_app_menu
import codex_discord_mirrored_busy_delegation as discord_mirrored_busy_delegation
import codex_discord_prompt_busy_result as discord_prompt_busy_result
import codex_discord_prompt_delivery_flow as discord_prompt_delivery_flow
import codex_discord_prompt_delivery_prepare as discord_prompt_delivery_prepare
import codex_discord_prompt_pending_delivery as discord_prompt_pending_delivery
import codex_discord_recorded_busy_transport as discord_recorded_busy_transport


ChannelT = TypeVar("ChannelT")
ChannelContraT = TypeVar("ChannelContraT", contravariant=True)
RelayT = TypeVar("RelayT", bound=discord_prompt_delivery_flow.PromptDeliveryRelay)
SendResultT = TypeVar("SendResultT")
SendResultCoT = TypeVar("SendResultCoT", covariant=True)


class AskDeliveryLock(Protocol):
    def locked(self) -> bool: ...

    async def __aenter__(self) -> None: ...

    async def __aexit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        traceback: TracebackType | None,
    ) -> bool | None: ...


class ChunkSender(Protocol[ChannelContraT, SendResultCoT]):
    def __call__(
        self,
        channel: ChannelContraT,
        text: str,
        *,
        context: str | None = None,
    ) -> Awaitable[SendResultCoT]: ...


class RecordedBusyHandler(Protocol[ChannelContraT]):
    def __call__(
        self,
        channel: ChannelContraT,
        prompt: str,
        *,
        target_thread_id: str | None,
        target_ref: str,
        recent_offsets: discord_prompt_busy_result.RecentOffsets,
        transport_output: str,
        delegate_to_session_mirror: bool,
        deps: discord_recorded_busy_transport.RecordedBusyTransportDeps,
    ) -> Awaitable[bool]: ...


class CodexAppMenuSender(Protocol[ChannelContraT]):
    def __call__(
        self,
        channel: ChannelContraT,
        target_thread_id: str | None,
        output: str,
        *,
        reason: str,
        deps: discord_codex_app_menu.CodexAppMenuDeps,
    ) -> Awaitable[bool]: ...


class PromptAdmission(Protocol[ChannelContraT]):
    def __call__(
        self,
        channel: ChannelContraT,
        source_message: object | None,
        expected_generation: int | None,
        /,
    ) -> AbstractAsyncContextManager[bool]: ...


@dataclass(frozen=True, slots=True)
class BotPromptDeliveryRuntimeDeps(Generic[ChannelT, RelayT, SendResultT]):
    resolve_target_ref: discord_prompt_delivery_prepare.TargetResolver
    get_ask_delivery_lock: Callable[[str | None], AskDeliveryLock]
    send_chunks: ChunkSender[ChannelT, SendResultT]
    build_ask_start_message: discord_prompt_delivery_prepare.AskStartMessageBuilder
    send_context_exhausted_prompt_notice_if_needed: discord_prompt_delivery_prepare.ContextExhaustionNotifier[
        ChannelT
    ]
    make_mapped_prompt_delivery_deps: discord_prompt_delivery_prepare.MappedDeliveryDepsFactory[
        ChannelT
    ]
    prepare_session_mirror_delegation: discord_prompt_delivery_prepare.SessionMirrorDelegationPreparer[
        ChannelT
    ]
    snapshot_ask_prompt_delivery_state: discord_prompt_delivery_prepare.DeliverySnapshotter
    prompt_delivery_bridge: discord_recorded_busy_transport.PromptDeliveryBridge
    get_delivery_confirm_timeout: discord_recorded_busy_transport.TimeoutGetter
    mark_optional_steering_handoff: discord_recorded_busy_transport.SteeringHandoffMarker
    stream_recorded_busy_steering_result: discord_recorded_busy_transport.SteeringResultStreamer
    get_pending_watch_timeout: discord_mirrored_busy_delegation.TimeoutGetter
    wait_for_codex_thread_idle: discord_mirrored_busy_delegation.CodexThreadIdleWaiter
    get_retry_attempts: Callable[[], int]
    get_retry_delay: Callable[[], float]
    monotonic: Callable[[], float]
    make_relay: Callable[
        [ChannelT, str | None, str, float, bool],
        RelayT,
    ]
    channel_typing: Callable[[ChannelT, str], AbstractAsyncContextManager[None]]
    run_ask_stream: Callable[[str, RelayT, str | None], Awaitable[tuple[int, str]]]
    is_discord_relay_stale: Callable[[str | None, int], bool]
    make_pending_delivery_deps: Callable[
        [],
        discord_prompt_pending_delivery.AskStreamPendingDeliveryDeps[ChannelT],
    ]
    sleep: Callable[[float], Awaitable[None]]
    make_busy_result_deps: Callable[
        [],
        discord_prompt_busy_result.AskStreamBusyResultDeps[ChannelT],
    ]
    send_prompt_chunks: discord_prompt_delivery_prepare.ChunkSender[ChannelT]
    had_steering_handoff_since: Callable[[str | None, float], bool]
    get_interactive_state_for_thread: discord_codex_app_menu.InteractiveStateGetter
    send_interactive_prompt: discord_codex_app_menu.InteractivePromptSender
    state_none: str
    state_input: str
    state_approval: str
    prompt_admission: PromptAdmission[ChannelT]
    format_log_text_len: Callable[[str | None], int]
    log: Callable[[str], None]
