from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from types import ModuleType
from typing import cast

import codex_discord_app_server_admission_factory as discord_app_server_admission_factory
import codex_discord_busy_choice_queue_action as discord_busy_choice_queue_action
import codex_discord_busy_choice_steer_action as discord_busy_choice_steer_action
import codex_discord_busy_choice_steer_failure as discord_busy_choice_steer_failure
import codex_discord_busy_choice_steer_result as discord_busy_choice_steer_result
import codex_discord_busy_choice_stop_action as discord_busy_choice_stop_action
import codex_discord_persistent_busy_choice as discord_persistent_busy_choice
import codex_discord_persistent_busy_queue as discord_persistent_busy_queue
import codex_discord_persistent_busy_steer as discord_persistent_busy_steer
import codex_discord_persistent_busy_steer_action as discord_persistent_busy_steer_action


@dataclass(frozen=True, slots=True)
class BotComponentDepsRuntime:
    module: ModuleType

    def make_persistent_busy_queue_deps(
        self,
    ) -> discord_persistent_busy_queue.PersistentBusyQueueActionDeps:
        module = self.module
        return discord_persistent_busy_queue.make_persistent_busy_queue_action_deps(
            get_busy_state_for_thread=cast(
                discord_persistent_busy_queue.SyncBusyStateGetter,
                getattr(module, "get_busy_state_for_thread"),
            ),
            is_thread_runner_busy=cast(
                discord_persistent_busy_queue.ThreadRunnerBusyChecker,
                getattr(module, "is_thread_runner_busy"),
            ),
            send_followup=cast(
                discord_persistent_busy_queue.BusyDirectFollowupSender,
                getattr(module, "send_busy_direct_followup"),
            ),
            enqueue_thread_ask=cast(
                discord_persistent_busy_queue.ThreadAskEnqueuer,
                getattr(module, "enqueue_thread_ask"),
            ),
            prompt_admission=discord_app_server_admission_factory.make_prompt_admission(
                module,
                cast(Callable[..., Awaitable[object]], getattr(module, "send_chunks")),
            ),
            handle_queue_followup=discord_persistent_busy_choice.handle_persistent_busy_queue_followup,
            format_log_text_len=cast(
                discord_persistent_busy_queue.LogTextLenFormatter,
                getattr(module, "format_log_text_len"),
            ),
            log=cast(discord_persistent_busy_queue.LogFunc, getattr(module, "log_line")),
        )

    def make_persistent_busy_steer_action_deps(
        self,
        steering_runner: discord_persistent_busy_steer.SteeringRunner,
        steering_streamer: discord_persistent_busy_steer.PersistentBusySteerStreamer,
    ) -> discord_persistent_busy_steer_action.PersistentBusySteerActionDeps:
        module = self.module
        discord_busy_module = cast(ModuleType, getattr(module, "discord_busy"))
        time_module = cast(ModuleType, getattr(module, "time"))
        return discord_persistent_busy_steer_action.make_persistent_busy_steer_action_deps(
            steering_runner=steering_runner,
            steering_streamer=steering_streamer,
            send_stale_block_message=cast(
                discord_persistent_busy_steer.BusyStaleSteerBlockSender,
                getattr(module, "send_busy_stale_block_message"),
            ),
            send_followup_chunks=cast(
                discord_persistent_busy_steer.BusyFollowupChunkSender,
                getattr(module, "send_busy_followup_chunks"),
            ),
            prepare_mapped_session_mirror_output=cast(
                discord_persistent_busy_steer.BusySteerSessionMirrorPreparer,
                getattr(module, "prepare_mapped_session_mirror_output"),
            ),
            prepare_session_mirror_delegation=cast(
                discord_persistent_busy_steer.BusySteerSessionMirrorPreparer,
                getattr(module, "prepare_session_mirror_delegation"),
            ),
            send_steering_start_ack=cast(
                discord_persistent_busy_steer_action.SteeringStartAckSender,
                getattr(module, "send_persistent_busy_steering_start_ack"),
            ),
            channel_typing=cast(
                discord_persistent_busy_steer.BusyChannelTypingFactory,
                getattr(module, "channel_typing"),
            ),
            mark_steering_handoff=cast(
                discord_persistent_busy_steer.SteeringHandoffMarker,
                getattr(module, "mark_persistent_busy_steering_handoff"),
            ),
            is_selected_thread_busy_error=cast(
                discord_persistent_busy_steer_action.BusyErrorDetector,
                getattr(discord_busy_module, "is_selected_thread_busy_error"),
            ),
            send_codex_app_menu_if_available=cast(
                discord_persistent_busy_steer.BusyCodexAppMenuSender,
                getattr(module, "send_busy_codex_app_menu_if_available"),
            ),
            resolve_target_ref=cast(discord_persistent_busy_steer.TargetRefResolver, getattr(module, "resolve_target_ref")),
            build_not_accepted_message=cast(
                discord_persistent_busy_steer.NotAcceptedMessageBuilder,
                getattr(module, "build_codex_app_steering_not_accepted_message"),
            ),
            format_log_text_len=cast(discord_persistent_busy_steer_action.TextLenFormatter, getattr(module, "format_log_text_len")),
            monotonic=cast(discord_persistent_busy_steer.MonotonicFunc, getattr(time_module, "monotonic")),
            log=cast(discord_persistent_busy_steer_action.LogFunc, getattr(module, "log_line")),
        )

    def make_busy_choice_steer_failure_deps(self) -> discord_busy_choice_steer_failure.BusyChoiceSteerFailureDeps:
        module = self.module
        return discord_busy_choice_steer_failure.BusyChoiceSteerFailureDeps(
            send_codex_app_menu_if_available=cast(
                discord_busy_choice_steer_failure.CodexAppMenuSender,
                getattr(module, "send_busy_codex_app_menu_if_available"),
            ),
            send_stale_block_message=cast(
                discord_busy_choice_steer_failure.StaleBusySteerBlockSender,
                getattr(module, "send_busy_stale_block_message"),
            ),
            send_followup_chunks=cast(
                discord_busy_choice_steer_failure.FollowupChunkSender,
                getattr(module, "send_busy_followup_chunks"),
            ),
            resolve_target_ref=cast(discord_busy_choice_steer_failure.TargetRefResolver, getattr(module, "resolve_target_ref")),
            build_not_accepted_message=cast(
                discord_busy_choice_steer_failure.NotAcceptedMessageBuilder,
                getattr(module, "build_codex_app_steering_not_accepted_message"),
            ),
            log=cast(discord_busy_choice_steer_failure.LogFunc, getattr(module, "log_line")),
        )

    def make_busy_choice_steer_result_deps(self) -> discord_busy_choice_steer_result.BusyChoiceSteerResultDeps:
        module = self.module
        return discord_busy_choice_steer_result.BusyChoiceSteerResultDeps(
            send_followup_chunks=cast(
                discord_busy_choice_steer_result.BusyChoiceFollowupChunkSender,
                getattr(module, "send_busy_followup_chunks"),
            ),
            steering_streamer=cast(
                discord_busy_choice_steer_result.BusyChoiceSteerStreamer,
                getattr(module, "stream_steering_prompt_result_to_channel"),
            ),
            log=cast(discord_busy_choice_steer_result.LogFunc, getattr(module, "log_line")),
        )

    def make_busy_choice_steer_action_deps(self) -> discord_busy_choice_steer_action.BusyChoiceSteerActionDeps:
        module = self.module
        time_module = cast(ModuleType, getattr(module, "time"))
        return discord_busy_choice_steer_action.BusyChoiceSteerActionDeps(
            send_stale_block_message=cast(
                discord_busy_choice_steer_action.StaleBusySteerBlockSender,
                getattr(module, "send_busy_stale_block_message"),
            ),
            prepare_mapped_session_mirror_output=cast(
                discord_busy_choice_steer_action.SessionMirrorOutputPreparer,
                getattr(module, "prepare_mapped_session_mirror_output"),
            ),
            prepare_session_mirror_delegation=cast(
                discord_busy_choice_steer_action.SessionMirrorOutputPreparer,
                getattr(module, "prepare_session_mirror_delegation"),
            ),
            send_steering_start_ack=cast(
                discord_busy_choice_steer_action.SteeringStartAckSender,
                getattr(module, "send_steering_start_ack"),
            ),
            send_followup_chunks=cast(
                discord_busy_choice_steer_action.FollowupChunkSender,
                getattr(module, "send_busy_followup_chunks"),
            ),
            channel_typing=cast(discord_busy_choice_steer_action.ChannelTypingFactory, getattr(module, "channel_typing")),
            run_steering_prompt=cast(discord_busy_choice_steer_action.SteeringRunner, getattr(module, "run_steering_prompt")),
            mark_steering_handoff=cast(discord_busy_choice_steer_action.SteeringHandoffMarker, getattr(module, "mark_steering_handoff")),
            format_log_text_len=cast(discord_busy_choice_steer_action.LogTextLenFormatter, getattr(module, "format_log_text_len")),
            log=cast(discord_busy_choice_steer_action.LogFunc, getattr(module, "log_line")),
            time_monotonic=cast(discord_busy_choice_steer_action.TimeNowFunc, getattr(time_module, "monotonic")),
            steer_failure_deps=self.make_busy_choice_steer_failure_deps(),
            steer_result_deps=self.make_busy_choice_steer_result_deps(),
        )

    def make_busy_choice_queue_action_deps(self) -> discord_busy_choice_queue_action.BusyChoiceQueueActionDeps:
        module = self.module
        asyncio_module = cast(ModuleType, getattr(module, "asyncio"))

        async def get_busy_state(target_thread_id: str | None) -> discord_busy_choice_queue_action.BusyState:
            to_thread = cast(
                Callable[..., Awaitable[discord_busy_choice_queue_action.BusyState]],
                getattr(asyncio_module, "to_thread"),
            )
            return await to_thread(getattr(module, "get_busy_state_for_thread"), target_thread_id)

        return discord_busy_choice_queue_action.BusyChoiceQueueActionDeps(
            get_busy_state_for_thread=get_busy_state,
            is_thread_runner_busy=cast(
                discord_busy_choice_queue_action.ThreadRunnerBusyChecker,
                getattr(module, "is_thread_runner_busy"),
            ),
            send_followup=cast(discord_busy_choice_queue_action.BusyDirectFollowupSender, getattr(module, "send_busy_direct_followup")),
            enqueue_thread_ask=cast(discord_busy_choice_queue_action.ThreadAskEnqueuer, getattr(module, "enqueue_thread_ask")),
            prompt_admission=discord_app_server_admission_factory.make_prompt_admission(
                module,
                cast(Callable[..., Awaitable[object]], getattr(module, "send_chunks")),
            ),
            format_log_text_len=cast(discord_busy_choice_queue_action.LogTextLenFormatter, getattr(module, "format_log_text_len")),
            log=cast(discord_busy_choice_queue_action.LogFunc, getattr(module, "log_line")),
        )

    def make_busy_choice_stop_action_deps(self) -> discord_busy_choice_stop_action.BusyChoiceStopActionDeps:
        module = self.module
        return discord_busy_choice_stop_action.BusyChoiceStopActionDeps(
            resolve_target_args=cast(
                discord_busy_choice_stop_action.ResolveTargetArgs,
                getattr(module, "resolve_discord_thread_target_args"),
            ),
            run_bridge_and_send=cast(
                discord_busy_choice_stop_action.BridgeRunner,
                getattr(module, "run_bridge_and_send"),
            ),
            send_direct_followup=cast(
                discord_busy_choice_stop_action.DirectFollowupSender,
                getattr(module, "send_busy_direct_followup"),
            ),
            log=cast(discord_busy_choice_stop_action.LogFunc, getattr(module, "log_line")),
        )
