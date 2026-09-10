from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
import time
from collections.abc import Callable
from contextlib import AbstractAsyncContextManager
from dataclasses import dataclass
from types import ModuleType
from typing import cast

import codex_discord_app_server_admission_factory as discord_app_server_admission_factory
import codex_discord_bot_prompt_delivery_runtime as discord_bot_prompt_delivery_runtime
import codex_discord_bot_prompt_delivery_types as discord_bot_prompt_delivery_types
import codex_discord_bot_shapes as discord_bot_shapes
from codex_discord_bot_prompt_delivery_adapter_types import (
    DiscordAskRelayMaker,
    ModuleValue,
    PromptChannel,
    PromptDeliveryChannelTyping,
    PromptRelay,
    RunAskStreamInThread,
    RuntimeConfigModule,
    SendResult,
)
import codex_discord_codex_app_menu as discord_codex_app_menu
import codex_discord_mirrored_busy_delegation as discord_mirrored_busy_delegation
import codex_discord_prompt_busy_result as discord_prompt_busy_result
import codex_discord_prompt_mapped_delivery as discord_prompt_mapped_delivery
import codex_discord_prompt_delivery_prepare as discord_prompt_delivery_prepare
import codex_discord_prompt_pending_delivery as discord_prompt_pending_delivery
import codex_discord_recorded_busy_transport as discord_recorded_busy_transport
from codex_thread_models import ThreadInfo


@dataclass(frozen=True, slots=True)
class BotPromptDeliveryAdapterRuntime:
    module: ModuleType

    def make_prompt_delivery_runtime(
        self,
    ) -> discord_bot_prompt_delivery_runtime.BotPromptDeliveryRuntime[
        PromptChannel,
        PromptRelay,
        SendResult,
    ]:
        return discord_bot_prompt_delivery_runtime.BotPromptDeliveryRuntime(
            discord_bot_prompt_delivery_types.BotPromptDeliveryRuntimeDeps(
                resolve_target_ref=self.resolve_target_ref,
                get_ask_delivery_lock=self.get_ask_delivery_lock,
                send_chunks=self.send_chunks,
                build_ask_start_message=self.build_ask_start_message,
                send_context_exhausted_prompt_notice_if_needed=cast(
                    discord_prompt_delivery_prepare.ContextExhaustionNotifier[PromptChannel],
                    self._module_func("send_context_exhausted_prompt_notice_if_needed"),
                ),
                make_mapped_prompt_delivery_deps=self.make_mapped_prompt_delivery_deps,
                prepare_session_mirror_delegation=self.prepare_session_mirror_delegation,
                snapshot_ask_prompt_delivery_state=self.snapshot_ask_prompt_delivery_state,
                prompt_delivery_bridge=cast(
                    discord_recorded_busy_transport.PromptDeliveryBridge,
                    getattr(self.module, "PROMPT_DELIVERY_BRIDGE"),
                ),
                get_delivery_confirm_timeout=cast(
                    discord_recorded_busy_transport.TimeoutGetter,
                    self._module_func("get_steering_delivery_confirm_timeout"),
                ),
                mark_optional_steering_handoff=cast(
                    discord_recorded_busy_transport.SteeringHandoffMarker,
                    self._module_func("mark_optional_steering_handoff"),
                ),
                stream_recorded_busy_steering_result=cast(
                    discord_recorded_busy_transport.SteeringResultStreamer,
                    self._module_func("stream_recorded_busy_steering_result"),
                ),
                get_pending_watch_timeout=self.get_pending_watch_timeout,
                wait_for_codex_thread_idle=cast(
                    discord_mirrored_busy_delegation.CodexThreadIdleWaiter,
                    self._module_func("wait_for_codex_thread_idle"),
                ),
                get_retry_attempts=self.get_retry_attempts,
                get_retry_delay=self.get_retry_delay,
                monotonic=time.monotonic,
                make_relay=self.make_relay,
                channel_typing=self.channel_typing,
                run_ask_stream=self.run_ask_stream,
                is_discord_relay_stale=cast(
                    Callable[[str | None, int], bool],
                    self._module_func("is_discord_relay_stale"),
                ),
                make_pending_delivery_deps=self.make_pending_delivery_deps,
                sleep=lambda seconds: asyncio.sleep(seconds),
                make_busy_result_deps=self.make_busy_result_deps,
                send_prompt_chunks=cast(
                    discord_prompt_delivery_prepare.ChunkSender[PromptChannel],
                    self._module_func("send_prompt_chunks"),
                ),
                had_steering_handoff_since=cast(
                    Callable[[str | None, float], bool],
                    self._module_func("had_steering_handoff_since"),
                ),
                get_interactive_state_for_thread=cast(
                    discord_codex_app_menu.InteractiveStateGetter,
                    self._module_func("get_interactive_state_for_thread"),
                ),
                send_interactive_prompt=cast(
                    discord_codex_app_menu.InteractivePromptSender,
                    self._module_func("send_interactive_prompt"),
                ),
                state_none=cast(str, getattr(self.module, "INTERACTIVE_STATE_NONE")),
                state_input=cast(str, getattr(self.module, "INTERACTIVE_STATE_INPUT")),
                state_approval=cast(str, getattr(self.module, "INTERACTIVE_STATE_APPROVAL")),
                prompt_admission=discord_app_server_admission_factory.make_prompt_admission(
                    self.module,
                    self.send_chunks,
                ),
                format_log_text_len=cast(Callable[[str | None], int], self._module_func("format_log_text_len")),
                log=cast(Callable[[str], None], self._module_func("log_line")),
            )
        )

    def resolve_target_ref(self, target_thread_id: str | None) -> tuple[str | None, str]:
        return cast(
            discord_prompt_delivery_prepare.TargetResolver,
            self._module_func("resolve_target_ref"),
        )(target_thread_id)

    def get_ask_delivery_lock(self, target_thread_id: str | None) -> discord_bot_prompt_delivery_types.AskDeliveryLock:
        return cast(
            Callable[[str | None], discord_bot_prompt_delivery_types.AskDeliveryLock],
            self._module_func("get_ask_delivery_lock"),
        )(target_thread_id)

    async def send_chunks(
        self,
        channel: PromptChannel,
        text: str,
        *,
        context: str | None = None,
    ) -> SendResult:
        return await cast(
            discord_bot_prompt_delivery_types.ChunkSender[PromptChannel, SendResult],
            self._module_func("send_chunks"),
        )(channel, text, context=context or "send_chunks")

    def build_ask_start_message(self, prompt: str, *, queued: bool = False) -> str:
        return cast(discord_prompt_delivery_prepare.AskStartMessageBuilder, self._module_func("build_ask_start_message"))(
            prompt,
            queued=queued,
        )

    def make_mapped_prompt_delivery_deps(
        self,
    ) -> discord_prompt_mapped_delivery.MappedPromptDeliveryDeps[PromptChannel]:
        return cast(
            discord_prompt_delivery_prepare.MappedDeliveryDepsFactory[PromptChannel],
            self._module_func("make_mapped_prompt_delivery_deps"),
        )()

    async def prepare_session_mirror_delegation(
        self,
        channel: PromptChannel,
        target_thread_id: str | None,
    ) -> bool:
        return await cast(
            discord_prompt_delivery_prepare.SessionMirrorDelegationPreparer[PromptChannel],
            self._module_func("prepare_session_mirror_delegation"),
        )(
            cast(discord_bot_shapes.SessionMirrorOutputChannel, channel),
            target_thread_id,
        )

    def snapshot_ask_prompt_delivery_state(
        self,
        target_thread_id: str | None,
    ) -> tuple[ThreadInfo | None, discord_prompt_busy_result.RecentOffsets]:
        return cast(
            discord_prompt_delivery_prepare.DeliverySnapshotter,
            self._module_func("snapshot_ask_prompt_delivery_state"),
        )(target_thread_id)

    def get_retry_attempts(self) -> int:
        runtime_config = cast(RuntimeConfigModule, getattr(self.module, "discord_runtime_config"))
        return runtime_config.get_ask_busy_retry_attempts(
            default=cast(float, getattr(self.module, "ASK_BUSY_RETRY_ATTEMPTS")),
        )

    def get_retry_delay(self) -> float:
        return cast(Callable[[], float], self._module_func("get_ask_busy_retry_delay_seconds"))()

    def get_pending_watch_timeout(self) -> float:
        return cast(
            discord_mirrored_busy_delegation.TimeoutGetter,
            self._module_func("get_steering_pending_watch_timeout"),
        )()

    def make_relay(
        self,
        channel: PromptChannel,
        target_thread_id: str | None,
        target_ref: str,
        started_at: float,
        delegate_to_session_mirror: bool,
    ) -> PromptRelay:
        return cast(
            DiscordAskRelayMaker,
            self._module_func("_make_discord_ask_relay"),
        )(
            channel,
            target_thread_id=target_thread_id,
            target_ref=target_ref,
            started_at=started_at,
            delegate_to_session_mirror=delegate_to_session_mirror,
        )

    def channel_typing(self, channel: PromptChannel, context: str) -> AbstractAsyncContextManager[None]:
        return cast(
            PromptDeliveryChannelTyping,
            self._module_func("prompt_delivery_channel_typing"),
        )(channel, context=context)

    async def run_ask_stream(
        self,
        prompt: str,
        relay: PromptRelay,
        target_thread_id: str | None,
    ) -> tuple[int, str]:
        return await cast(
            RunAskStreamInThread,
            self._module_func("_run_ask_stream_in_thread"),
        )(prompt, relay, target_thread_id=target_thread_id)

    def make_pending_delivery_deps(self) -> discord_prompt_pending_delivery.AskStreamPendingDeliveryDeps[PromptChannel]:
        return cast(
            Callable[[], discord_prompt_pending_delivery.AskStreamPendingDeliveryDeps[PromptChannel]],
            self._module_func("make_ask_stream_pending_delivery_deps"),
        )()

    def make_busy_result_deps(self) -> discord_prompt_busy_result.AskStreamBusyResultDeps[PromptChannel]:
        return cast(
            Callable[[], discord_prompt_busy_result.AskStreamBusyResultDeps[PromptChannel]],
            self._module_func("make_ask_stream_busy_result_deps"),
        )()

    def _module_func(self, name: str) -> ModuleValue:
        return cast(ModuleValue, getattr(self.module, name))
