from __future__ import annotations

from collections.abc import Callable
from contextlib import AbstractContextManager
from dataclasses import dataclass
from typing import Generic, TypeVar

import codex_app_server_transport as app_server_transport
import codex_app_server_transport_delivery as app_server_delivery
import codex_discord_ask_stream_factory as discord_ask_stream_factory
import codex_discord_prompt_busy_result as discord_prompt_busy_result
import codex_discord_prompt_mapped_delivery as discord_prompt_mapped_delivery
import codex_discord_prompt_mapped_delivery_factory as discord_prompt_mapped_delivery_factory
import codex_discord_prompt_pending_delivery as discord_prompt_pending_delivery
import codex_discord_prompt_transport as discord_prompt_transport
import codex_discord_prompt_transport_factory as discord_prompt_transport_factory
import codex_discord_stream as discord_stream


ChannelT = TypeVar("ChannelT")
RelayT = TypeVar("RelayT", bound=discord_stream.AskStreamRelay)
SteeringResultT = TypeVar("SteeringResultT")


@dataclass(frozen=True, slots=True)
class BotPromptTransportRuntimeDeps(Generic[ChannelT, RelayT, SteeringResultT]):
    bridge_module: app_server_delivery.BridgeModule
    app_server_transport_enabled: discord_prompt_transport.TransportEnabled
    run_legacy_prompt_no_wait: discord_prompt_transport.PromptNoWait
    make_steering_prompt_result: discord_prompt_transport.MakeSteeringResult[
        app_server_transport.AppServerDeliveryResult,
        SteeringResultT,
    ]
    run_watch_stream: discord_prompt_transport.WatchStream[SteeringResultT, RelayT]
    run_bridge_command_stream: discord_stream.RunBridgeCommandStreamFunc
    ui_fallback_lock: AbstractContextManager[bool]
    preprocess_prompt: discord_prompt_mapped_delivery.PromptPreprocessor
    mark_recent_discord_origin_prompt: discord_prompt_mapped_delivery.DiscordOriginPromptMarker
    prepare_mapped_session_mirror_output: discord_prompt_mapped_delivery.PrepareMappedSessionMirrorOutput[ChannelT]
    set_selected_thread_id: discord_prompt_mapped_delivery.SelectedThreadSetter
    channel_typing: discord_prompt_mapped_delivery.ChannelTyping[ChannelT]
    run_transport_prompt_no_wait: discord_prompt_mapped_delivery_factory.SyncTransportNoWait
    send_chunks: discord_prompt_mapped_delivery.ChunkSender[ChannelT]
    is_delivery_confirmation_timeout: discord_prompt_mapped_delivery.OutputPredicate
    format_pending_ask_delivery_output: discord_prompt_mapped_delivery.PendingFormatter
    release_session_mirror_output_target: discord_prompt_mapped_delivery.OutputTargetReleaser
    is_selected_thread_busy_error: discord_prompt_mapped_delivery.BusyPredicate
    send_codex_app_menu_if_available: discord_prompt_mapped_delivery.AppMenuSender[ChannelT]
    send_resume_failure: discord_prompt_mapped_delivery.ResumeFailureSender[ChannelT]
    handle_recorded_busy_transport_prompt: discord_prompt_busy_result.RecordedBusyHandler[ChannelT]
    wait_for_mirrored_busy_delegation_settle: discord_prompt_busy_result.BusySettleWaiter
    mark_steering_handoff: discord_prompt_busy_result.SteeringHandoffMarker
    get_relay_factory: Callable[[], discord_ask_stream_factory.DiscordAskRelayFactory[ChannelT, RelayT]]
    get_run_ask_stream: Callable[[], discord_ask_stream_factory.RunAskStreamFunc[RelayT]]
    format_log_text_len: discord_prompt_busy_result.TextLenFunc
    log: discord_prompt_busy_result.LogFunc


@dataclass(frozen=True, slots=True)
class BotPromptTransportRuntime(Generic[ChannelT, RelayT, SteeringResultT]):
    deps: BotPromptTransportRuntimeDeps[ChannelT, RelayT, SteeringResultT]

    def make_prompt_transport_deps(
        self,
    ) -> discord_prompt_transport.PromptTransportDeps[RelayT, app_server_transport.AppServerDeliveryResult, SteeringResultT]:
        return discord_prompt_transport_factory.make_prompt_transport_deps(
            bridge_module=self.deps.bridge_module,
            app_server_transport_enabled=self.deps.app_server_transport_enabled,
            run_legacy_prompt_no_wait=self.deps.run_legacy_prompt_no_wait,
            make_steering_prompt_result=self.deps.make_steering_prompt_result,
            run_watch_stream=self.deps.run_watch_stream,
            run_bridge_command_stream=self.deps.run_bridge_command_stream,
            ui_fallback_lock=self.deps.ui_fallback_lock,
            log=self.deps.log,
        )

    def make_mapped_prompt_delivery_deps(self) -> discord_prompt_mapped_delivery.MappedPromptDeliveryDeps[ChannelT]:
        return discord_prompt_mapped_delivery_factory.make_mapped_prompt_delivery_deps(
            prepare_mapped_session_mirror_output=self.deps.prepare_mapped_session_mirror_output,
            set_selected_thread_id=self.deps.set_selected_thread_id,
            channel_typing=self.deps.channel_typing,
            preprocess_prompt=self.deps.preprocess_prompt,
            mark_recent_discord_origin_prompt=self.deps.mark_recent_discord_origin_prompt,
            run_transport_prompt_no_wait=self.deps.run_transport_prompt_no_wait,
            send_chunks=self.deps.send_chunks,
            is_delivery_confirmation_timeout=self.deps.is_delivery_confirmation_timeout,
            format_pending_ask_delivery_output=self.deps.format_pending_ask_delivery_output,
            release_session_mirror_output_target=self.deps.release_session_mirror_output_target,
            is_selected_thread_busy_error=self.deps.is_selected_thread_busy_error,
            send_codex_app_menu_if_available=self.deps.send_codex_app_menu_if_available,
            send_resume_failure=self.deps.send_resume_failure,
            format_log_text_len=self.deps.format_log_text_len,
            log=self.deps.log,
        )

    def make_ask_stream_pending_delivery_deps(
        self,
    ) -> discord_prompt_pending_delivery.AskStreamPendingDeliveryDeps[ChannelT]:
        return discord_ask_stream_factory.make_ask_stream_pending_delivery_deps(
            is_delivery_confirmation_timeout=self.deps.is_delivery_confirmation_timeout,
            send_chunks=self.deps.send_chunks,
            format_log_text_len=self.deps.format_log_text_len,
            log=self.deps.log,
        )

    def make_discord_ask_relay(
        self,
        channel: ChannelT,
        *,
        target_thread_id: str | None,
        target_ref: str,
        started_at: float,
        delegate_to_session_mirror: bool,
    ) -> RelayT:
        return discord_ask_stream_factory.make_discord_ask_relay(
            self.deps.get_relay_factory(),
            channel,
            target_thread_id=target_thread_id,
            target_ref=target_ref,
            started_at=started_at,
            delegate_to_session_mirror=delegate_to_session_mirror,
        )

    async def run_ask_stream_in_thread(
        self,
        prompt: str,
        relay: RelayT,
        *,
        target_thread_id: str | None,
    ) -> tuple[int, str]:
        return await discord_ask_stream_factory.run_ask_stream_in_thread(
            self.deps.get_run_ask_stream(),
            prompt,
            relay,
            target_thread_id=target_thread_id,
        )

    def make_ask_stream_busy_result_deps(self) -> discord_prompt_busy_result.AskStreamBusyResultDeps[ChannelT]:
        return discord_ask_stream_factory.make_ask_stream_busy_result_deps(
            handle_recorded_busy_transport_prompt=self.deps.handle_recorded_busy_transport_prompt,
            wait_for_mirrored_busy_delegation_settle=self.deps.wait_for_mirrored_busy_delegation_settle,
            mark_steering_handoff=self.deps.mark_steering_handoff,
            send_codex_app_menu_if_available=self.deps.send_codex_app_menu_if_available,
            format_log_text_len=self.deps.format_log_text_len,
            log=self.deps.log,
        )

    def run_ask_stream(
        self,
        prompt: str,
        relay: RelayT,
        *,
        force_while_busy: bool = False,
        wait: bool = True,
        target_thread_id: str | None = None,
    ) -> tuple[int, str]:
        return discord_prompt_transport.run_ask_stream(
            prompt,
            relay,
            force_while_busy=force_while_busy,
            wait=wait,
            target_thread_id=target_thread_id,
            deps=self.make_prompt_transport_deps(),
        )
