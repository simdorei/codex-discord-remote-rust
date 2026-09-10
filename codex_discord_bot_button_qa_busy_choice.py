from __future__ import annotations

from collections.abc import Awaitable
from types import ModuleType
from typing import Protocol, cast, final, override

import codex_discord_bot_component_deps_runtime as discord_bot_component_deps_runtime
import codex_discord_bot_persistent_busy_component_runtime as discord_bot_persistent_busy_component_runtime
from codex_discord_bot_button_qa_adapter_types import (
    QaChannel,
    QaInteraction,
    QaInteractionResolver,
)
import codex_discord_button_qa_lifecycle_cases as discord_button_qa_lifecycle_cases
import codex_discord_button_qa_steer_case as discord_button_qa_steer_case
import codex_discord_persistent_busy_steer_action as discord_persistent_busy_steer_action


class ModuleAttribute(Protocol):
    pass


class QaStaleBlockSender(Protocol):
    def __call__(
        self,
        channel: QaChannel,
        prompt: str,
        target_thread_id: str | None,
        *,
        reason: str,
    ) -> Awaitable[bool]: ...


@final
class QaNoStaleBlockModule(ModuleType):
    _module: ModuleType
    send_busy_stale_block_message: QaStaleBlockSender

    def __init__(self, module: ModuleType, qa_no_stale_block: QaStaleBlockSender) -> None:
        super().__init__(module.__name__, module.__doc__)
        self.__package__ = module.__package__
        self.__loader__ = module.__loader__
        self.__spec__ = module.__spec__
        self._module = module
        self.send_busy_stale_block_message = qa_no_stale_block
        setattr(
            self,
            "_make_persistent_busy_steer_action_deps",
            self.make_persistent_busy_steer_action_deps,
        )

    @override
    def __getattr__(self, name: str) -> ModuleAttribute:
        return cast(ModuleAttribute, getattr(self._module, name))

    def make_persistent_busy_steer_action_deps(
        self,
        steering_runner: discord_button_qa_steer_case.SteeringRunner,
        steering_streamer: discord_button_qa_steer_case.SteeringStreamer,
    ) -> discord_persistent_busy_steer_action.PersistentBusySteerActionDeps:
        return discord_bot_component_deps_runtime.BotComponentDepsRuntime(
            self,
        ).make_persistent_busy_steer_action_deps(steering_runner, steering_streamer)


class BotButtonQaBusyChoiceMixin(Protocol):
    module: ModuleType

    async def _handle_persistent_busy_choice_interaction(
        self,
        interaction: discord_button_qa_lifecycle_cases.BusyChoiceQaInteraction
        | discord_button_qa_steer_case.SteerQaInteraction,
        custom_id: str,
        *,
        steering_runner: discord_button_qa_steer_case.SteeringRunner | None = None,
        steering_streamer: discord_button_qa_steer_case.SteeringStreamer | None = None,
    ) -> bool:
        if steering_runner is None or steering_streamer is None:
            lifecycle_handler = cast(
                discord_button_qa_lifecycle_cases.BusyChoiceInteractionHandler,
                getattr(self.module, "handle_persistent_busy_choice_interaction"),
            )
            return await lifecycle_handler(
                cast(
                    discord_button_qa_lifecycle_cases.BusyChoiceQaInteraction,
                    self._require_discord_interaction(interaction),
                ),
                custom_id,
            )
        return await self._call_qa_steer_without_stale_block(
            cast(discord_button_qa_steer_case.SteerQaInteraction, self._require_discord_interaction(interaction)),
            custom_id,
            steering_runner=steering_runner,
            steering_streamer=steering_streamer,
        )

    async def _call_qa_steer_without_stale_block(
        self,
        interaction: discord_button_qa_steer_case.SteerQaInteraction,
        custom_id: str,
        *,
        steering_runner: discord_button_qa_steer_case.SteeringRunner,
        steering_streamer: discord_button_qa_steer_case.SteeringStreamer,
    ) -> bool:
        qa_module = QaNoStaleBlockModule(self.module, self._qa_no_stale_block)
        runtime = discord_bot_persistent_busy_component_runtime.BotPersistentBusyComponentRuntime(
            qa_module,
        )
        return await runtime.handle_persistent_busy_choice_interaction(
            interaction,
            custom_id,
            steering_runner=steering_runner,
            steering_streamer=steering_streamer,
        )

    async def _qa_no_stale_block(
        self,
        channel: QaChannel,
        prompt: str,
        target_thread_id: str | None,
        *,
        reason: str,
    ) -> bool:
        _ = (channel, prompt, target_thread_id, reason)
        return False

    def _require_discord_interaction(self, interaction: QaInteraction) -> QaInteraction:
        resolver = cast(QaInteractionResolver, getattr(self.module, "require_discord_interaction"))
        return resolver(interaction)
