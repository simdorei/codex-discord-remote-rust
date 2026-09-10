from __future__ import annotations

from contextlib import AbstractAsyncContextManager
from dataclasses import dataclass
from typing import Generic, cast

from codex_discord_bot_prompt_delivery_types import (
    BotPromptDeliveryRuntimeDeps,
    ChannelT,
    CodexAppMenuSender,
    RecordedBusyHandler,
    RelayT,
    SendResultT,
)
import codex_discord_busy as discord_busy
import codex_discord_codex_app_menu as discord_codex_app_menu
import codex_discord_mirrored_busy_delegation as discord_mirrored_busy_delegation
import codex_discord_prompt_busy_result as discord_prompt_busy_result
import codex_discord_prompt_delivery_flow as discord_prompt_delivery_flow
import codex_discord_prompt_delivery_prepare as discord_prompt_delivery_prepare
import codex_discord_recorded_busy_transport as discord_recorded_busy_transport
@dataclass(frozen=True, slots=True)
class BotPromptDeliveryRuntime(Generic[ChannelT, RelayT, SendResultT]):
    deps: BotPromptDeliveryRuntimeDeps[ChannelT, RelayT, SendResultT]

    def make_prompt_delivery_preparation_deps(
        self,
    ) -> discord_prompt_delivery_prepare.PromptDeliveryPreparationDeps[ChannelT]:
        async def send_start_chunks(
            channel: ChannelT,
            content: str,
            *,
            context: str | None = None,
        ) -> None:
            _ = await self.deps.send_chunks(channel, content, context=context)

        return discord_prompt_delivery_prepare.PromptDeliveryPreparationDeps(
            send_chunks=send_start_chunks,
            build_ask_start_message=self.deps.build_ask_start_message,
            resolve_target_ref=self.deps.resolve_target_ref,
            send_context_exhausted_prompt_notice_if_needed=(
                self.deps.send_context_exhausted_prompt_notice_if_needed
            ),
            make_mapped_prompt_delivery_deps=self.deps.make_mapped_prompt_delivery_deps,
            prepare_session_mirror_delegation=self.deps.prepare_session_mirror_delegation,
            snapshot_ask_prompt_delivery_state=self.deps.snapshot_ask_prompt_delivery_state,
        )
    async def handle_recorded_busy_transport_prompt(
        self,
        channel: ChannelT,
        prompt: str,
        *,
        target_thread_id: str | None,
        target_ref: str,
        recent_offsets: discord_prompt_busy_result.RecentOffsets,
        transport_output: str,
        delegate_to_session_mirror: bool,
    ) -> bool:
        handler = cast(
            RecordedBusyHandler[ChannelT],
            discord_recorded_busy_transport.handle_recorded_busy_transport_prompt,
        )
        return await handler(
            channel,
            prompt,
            target_thread_id=target_thread_id,
            target_ref=target_ref,
            recent_offsets=recent_offsets,
            transport_output=transport_output,
            delegate_to_session_mirror=delegate_to_session_mirror,
            deps=discord_recorded_busy_transport.RecordedBusyTransportDeps(
                bridge=self.deps.prompt_delivery_bridge,
                get_delivery_confirm_timeout=self.deps.get_delivery_confirm_timeout,
                mark_steering_handoff=self.deps.mark_optional_steering_handoff,
                stream_steering_prompt_result_to_channel=self.deps.stream_recorded_busy_steering_result,
                format_log_text_len=self.deps.format_log_text_len,
                log=self.deps.log,
            ),
        )
    async def wait_for_mirrored_busy_delegation_settle(
        self,
        prompt: str,
        *,
        target_thread_id: str | None,
        recent_offsets: discord_prompt_busy_result.RecentOffsets,
    ) -> None:
        await discord_mirrored_busy_delegation.wait_for_mirrored_busy_delegation_settle(
            prompt,
            target_thread_id=target_thread_id,
            recent_offsets=recent_offsets,
            deps=discord_mirrored_busy_delegation.MirroredBusyDelegationDeps(
                bridge=self.deps.prompt_delivery_bridge,
                get_pending_watch_timeout=self.deps.get_pending_watch_timeout,
                wait_for_codex_thread_idle=self.deps.wait_for_codex_thread_idle,
                log=self.deps.log,
            ),
        )
    async def run_prompt_and_send(
        self,
        channel: ChannelT,
        prompt: str,
        *,
        queued: bool = False,
        ack_sent: bool = False,
        source_message: discord_busy.BusyChoiceSource | None = None,
        target_thread_id: str | None = None,
        expected_app_server_generation: int | None = None,
    ) -> discord_prompt_delivery_prepare.PromptDeliveryPreparationResult:
        async with self.deps.prompt_admission(
            channel,
            source_message,
            expected_app_server_generation,
        ) as accepted:
            target_thread_id, target_ref = self.deps.resolve_target_ref(target_thread_id)
            if not accepted:
                return discord_prompt_delivery_prepare.discarded_prompt_delivery(
                    target_thread_id,
                    target_ref,
                )
            lock = self.deps.get_ask_delivery_lock(target_thread_id)
            waited = lock.locked()
            if waited:
                self.deps.log(f"ask_delivery_wait target={target_thread_id or '-'}")
            async with lock:
                if waited:
                    self.deps.log(f"ask_delivery_wait_done target={target_thread_id or '-'}")
                return await self._run_prompt_and_send_unlocked(
                    channel,
                    prompt,
                    queued=queued,
                    ack_sent=ack_sent,
                    source_message=source_message,
                    target_thread_id=target_thread_id,
                )
    async def send_codex_app_menu_if_available(
        self,
        channel: ChannelT,
        target_thread_id: str | None,
        output: str,
        *,
        reason: str,
    ) -> bool:
        sender = cast(
            CodexAppMenuSender[ChannelT],
            discord_codex_app_menu.send_codex_app_menu_if_available,
        )
        return await sender(
            channel,
            target_thread_id,
            output,
            reason=reason,
            deps=discord_codex_app_menu.CodexAppMenuDeps(
                get_interactive_state_for_thread=self.deps.get_interactive_state_for_thread,
                resolve_target_ref=self.deps.resolve_target_ref,
                send_interactive_prompt=self.deps.send_interactive_prompt,
                state_none=self.deps.state_none,
                state_input=self.deps.state_input,
                state_approval=self.deps.state_approval,
                log=self.deps.log,
            ),
        )

    async def _run_prompt_and_send_unlocked(
        self,
        channel: ChannelT,
        prompt: str,
        *,
        queued: bool = False,
        ack_sent: bool = False,
        source_message: discord_busy.BusyChoiceSource | None = None,
        target_thread_id: str | None = None,
    ) -> discord_prompt_delivery_prepare.PromptDeliveryPreparationResult:
        preparation = await discord_prompt_delivery_prepare.prepare_prompt_delivery(
            channel,
            discord_prompt_delivery_prepare.PromptDeliveryPreparationRequest(
                prompt=prompt,
                queued=queued,
                ack_sent=ack_sent,
                target_thread_id=target_thread_id,
            ),
            deps=self.make_prompt_delivery_preparation_deps(),
        )
        if preparation.handled:
            return preparation

        async def send_retry_notice_chunks(channel: ChannelT, text: str) -> SendResultT:
            return await self.deps.send_chunks(channel, text)

        def make_relay(
            channel: ChannelT,
            *,
            target_thread_id: str | None,
            target_ref: str,
            started_at: float,
            delegate_to_session_mirror: bool,
        ) -> RelayT:
            return self.deps.make_relay(
                channel,
                target_thread_id,
                target_ref,
                started_at,
                delegate_to_session_mirror,
            )

        def channel_typing(
            channel: ChannelT,
            *,
            context: str,
        ) -> AbstractAsyncContextManager[None]:
            return self.deps.channel_typing(channel, context)

        async def run_ask_stream(
            prompt: str,
            relay: RelayT,
            *,
            target_thread_id: str | None,
        ) -> tuple[int, str]:
            return await self.deps.run_ask_stream(prompt, relay, target_thread_id)

        await discord_prompt_delivery_flow.run_prompt_delivery_flow(
            channel,
            prompt=prompt,
            target_thread_id=preparation.target_thread_id,
            target_ref=preparation.target_ref,
            recent_offsets=preparation.recent_offsets,
            delegate_to_session_mirror=preparation.delegate_to_session_mirror,
            retry_attempts=self.deps.get_retry_attempts(),
            retry_delay=self.deps.get_retry_delay(),
            source_message_available=discord_busy.has_busy_choice_source(source_message),
            deps=discord_prompt_delivery_flow.make_prompt_delivery_flow_deps(
                monotonic=self.deps.monotonic,
                make_relay=make_relay,
                channel_typing=channel_typing,
                run_ask_stream=run_ask_stream,
                is_discord_relay_stale=self.deps.is_discord_relay_stale,
                pending_delivery_deps=self.deps.make_pending_delivery_deps(),
                is_selected_thread_busy_error=discord_busy.is_selected_thread_busy_error,
                sleep=self.deps.sleep,
                busy_result_deps=self.deps.make_busy_result_deps(),
                send_retry_notice_chunks=send_retry_notice_chunks,
                send_status_chunks=self.deps.send_prompt_chunks,
                build_codex_app_busy_retry_message=discord_prompt_busy_result.build_codex_app_busy_retry_message,
                send_result_chunks=self.deps.send_prompt_chunks,
                had_steering_handoff_since=self.deps.had_steering_handoff_since,
                format_log_text_len=self.deps.format_log_text_len,
                log=self.deps.log,
            ),
        )
        return preparation
