from __future__ import annotations

from dataclasses import dataclass
from types import ModuleType
from typing import Protocol, cast

import codex_discord_persistent_busy_choice as discord_persistent_busy_choice
import codex_discord_persistent_busy_choice_interaction as discord_persistent_busy_choice_interaction
import codex_discord_persistent_busy_queue as discord_persistent_busy_queue
import codex_discord_persistent_busy_steer as discord_persistent_busy_steer
import codex_discord_persistent_busy_steer_action as discord_persistent_busy_steer_action
import codex_discord_busy_choice_stop_action as discord_busy_choice_stop_action


class PersistentBusySteerDepsFactory(Protocol):
    def __call__(
        self,
        steering_runner: discord_persistent_busy_steer.SteeringRunner,
        steering_streamer: discord_persistent_busy_steer.PersistentBusySteerStreamer,
    ) -> discord_persistent_busy_steer_action.PersistentBusySteerActionDeps: ...


class PersistentBusyQueueDepsFactory(Protocol):
    def __call__(self) -> discord_persistent_busy_queue.PersistentBusyQueueActionDeps: ...


class PersistentBusyStopDepsFactory(Protocol):
    def __call__(self) -> discord_busy_choice_stop_action.BusyChoiceStopActionDeps: ...


@dataclass(frozen=True, slots=True)
class BotPersistentBusyComponentRuntime:
    module: ModuleType

    async def clear_busy_interaction_components(
        self,
        interaction: discord_persistent_busy_choice.PersistentBusyInteraction,
        *,
        context: str,
    ) -> None:
        clear_components = cast(
            discord_persistent_busy_choice_interaction.BusyComponentClearer,
            getattr(self.module, "clear_interaction_message_components"),
        )
        await clear_components(interaction, context=context)

    async def send_busy_interaction_response(
        self,
        interaction: discord_persistent_busy_choice.PersistentBusyInteraction,
        content: str,
        *,
        context: str,
    ) -> None:
        send_response = cast(
            discord_persistent_busy_choice_interaction.BusySimpleResponseSender,
            getattr(self.module, "send_interaction_response_tracked"),
        )
        await send_response(interaction, content, context=context)

    async def handle_persistent_busy_choice_interaction(
        self,
        interaction: discord_persistent_busy_choice_interaction.PersistentBusyInteraction,
        custom_id: str,
        *,
        steering_runner: discord_persistent_busy_steer.SteeringRunner | None = None,
        steering_streamer: discord_persistent_busy_steer.PersistentBusySteerStreamer | None = None,
    ) -> bool:
        run_steering = steering_runner
        if run_steering is None:
            run_steering = cast(discord_persistent_busy_steer.SteeringRunner, getattr(self.module, "run_steering_prompt"))
        stream_steering = steering_streamer
        if stream_steering is None:
            stream_steering = cast(
                discord_persistent_busy_steer.PersistentBusySteerStreamer,
                getattr(self.module, "stream_steering_prompt_result_to_channel"),
            )
        return await discord_persistent_busy_choice_interaction.handle_persistent_busy_choice_interaction(
            interaction,
            custom_id,
            deps=discord_persistent_busy_choice_interaction.PersistentBusyChoiceInteractionDeps(
                get_busy_choice_record=cast(
                    discord_persistent_busy_choice_interaction.BusyChoiceRecordGetter,
                    getattr(self.module, "get_busy_choice_record"),
                ),
                claim_busy_choice_record=cast(
                    discord_persistent_busy_choice_interaction.BusyChoiceRecordClaimer,
                    getattr(self.module, "claim_busy_choice_record"),
                ),
                clear_interaction_message_components=cast(
                    discord_persistent_busy_choice_interaction.BusyComponentClearer,
                    getattr(self.module, "clear_interaction_message_components"),
                ),
                send_interaction_response=cast(
                    discord_persistent_busy_choice_interaction.BusyResponseSender,
                    getattr(self.module, "send_interaction_response_tracked"),
                ),
                clear_busy_interaction_components=self.clear_busy_interaction_components,
                send_busy_interaction_response=self.send_busy_interaction_response,
                send_busy_direct_followup=cast(
                    discord_persistent_busy_choice_interaction.BusyDirectFollowupSender,
                    getattr(self.module, "send_busy_direct_followup"),
                ),
                resolve_interaction_channel=cast(
                    discord_persistent_busy_choice_interaction.InteractionChannelResolver,
                    getattr(self.module, "resolve_interaction_channel"),
                ),
                steer_action_deps=self._make_steer_deps(run_steering, stream_steering),
                queue_action_deps=self._make_queue_deps(),
                stop_action_deps=self._make_stop_deps(),
                log=cast(discord_persistent_busy_choice_interaction.LogFunc, getattr(self.module, "log_line")),
            ),
        )

    def _make_steer_deps(
        self,
        steering_runner: discord_persistent_busy_steer.SteeringRunner,
        steering_streamer: discord_persistent_busy_steer.PersistentBusySteerStreamer,
    ) -> discord_persistent_busy_steer_action.PersistentBusySteerActionDeps:
        make_deps = cast(
            PersistentBusySteerDepsFactory,
            getattr(self.module, "_make_persistent_busy_steer_action_deps"),
        )
        return make_deps(steering_runner, steering_streamer)

    def _make_queue_deps(self) -> discord_persistent_busy_queue.PersistentBusyQueueActionDeps:
        make_deps = cast(
            PersistentBusyQueueDepsFactory,
            getattr(self.module, "_make_persistent_busy_queue_deps"),
        )
        return make_deps()

    def _make_stop_deps(self) -> discord_busy_choice_stop_action.BusyChoiceStopActionDeps:
        make_deps = cast(
            PersistentBusyStopDepsFactory,
            getattr(self.module, "_make_busy_choice_stop_action_deps"),
        )
        return make_deps()
