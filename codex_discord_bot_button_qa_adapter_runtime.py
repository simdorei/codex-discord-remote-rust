from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from types import ModuleType
from typing import cast

import codex_discord_bot_button_qa_runtime as discord_bot_button_qa_runtime
from codex_discord_bot_button_qa_adapter_types import (
    QaChannel,
    QaMessageableResolver,
    QaSendMessageTracked,
    QaView,
    RuntimeSlashBotTypeError,
)
from codex_discord_bot_button_qa_busy_choice import BotButtonQaBusyChoiceMixin
import codex_discord_button_qa_cases as discord_button_qa_cases
import codex_discord_button_qa_lifecycle_cases as discord_button_qa_lifecycle_cases
import codex_discord_button_qa_persistent_cases as discord_button_qa_persistent_cases
import codex_discord_button_qa_steer_case as discord_button_qa_steer_case
import codex_discord_prefix_qa_command as discord_prefix_qa_command
import codex_discord_slash_commands as discord_slash_commands
import codex_discord_slash_runtime_commands as discord_slash_runtime_commands


@dataclass(frozen=True, slots=True)
class BotButtonQaAdapterRuntime(BotButtonQaBusyChoiceMixin):
    module: ModuleType

    async def send_busy_choice_qa_message_tracked(
        self,
        channel: discord_button_qa_cases.ButtonQaChannel,
        content: str,
        *,
        view: discord_button_qa_cases.ViewLike | None = None,
        context: str = "send_message_tracked",
    ) -> discord_button_qa_cases.SentMessageLike:
        sent_message = await self._send_message_tracked(channel, content, view=view, context=context)
        return sent_message

    async def send_persistent_button_qa_message_tracked(
        self,
        channel: discord_button_qa_persistent_cases.PersistentQaChannel,
        content: str,
        *,
        view: discord_button_qa_persistent_cases.ViewLike | None = None,
        context: str = "send_message_tracked",
    ) -> discord_button_qa_persistent_cases.PersistentQaMessage:
        sent_message = await self._send_message_tracked(channel, content, view=view, context=context)
        return cast(discord_button_qa_persistent_cases.PersistentQaMessage, sent_message)

    def make_button_qa_busy_choice_payload(
        self,
        source_message: discord_button_qa_cases.ButtonQaMessage,
        prompt: str,
        *,
        target_thread_id: str | None = None,
        allow_steer: bool = True,
    ) -> tuple[str, discord_button_qa_cases.ViewLike]:
        make_payload = cast(discord_button_qa_cases.MakeBusyChoicePayloadFunc, getattr(self.module, "make_busy_choice_payload"))
        content, view = make_payload(
            source_message,
            prompt,
            target_thread_id=target_thread_id,
            allow_steer=allow_steer,
        )
        return content, cast(discord_button_qa_cases.ViewLike, cast(object, view))

    async def handle_lifecycle_qa_busy_choice_interaction(
        self,
        interaction: discord_button_qa_lifecycle_cases.BusyChoiceQaInteraction,
        custom_id: str,
    ) -> bool:
        return await self._handle_persistent_busy_choice_interaction(interaction, custom_id)

    async def handle_steer_qa_busy_choice_interaction(
        self,
        interaction: discord_button_qa_steer_case.SteerQaInteraction,
        custom_id: str,
        *,
        steering_runner: discord_button_qa_steer_case.SteeringRunner,
        steering_streamer: discord_button_qa_steer_case.SteeringStreamer,
    ) -> bool:
        return await self._handle_persistent_busy_choice_interaction(
            interaction,
            custom_id,
            steering_runner=steering_runner,
            steering_streamer=steering_streamer,
        )

    def make_persistent_qa_approval_view(self, target_thread_id: str) -> discord_button_qa_persistent_cases.ViewLike:
        approval_view_type = cast(type, getattr(self.module, "ApprovalView"))
        return cast(discord_button_qa_persistent_cases.ViewLike, cast(object, approval_view_type(target_thread_id)))

    def make_persistent_qa_input_choice_view(
        self,
        target_thread_id: str,
        options: list[tuple[str, str]],
    ) -> discord_button_qa_persistent_cases.ViewLike:
        input_view_type = cast(type, getattr(self.module, "InputChoiceView"))
        return cast(discord_button_qa_persistent_cases.ViewLike, cast(object, input_view_type(target_thread_id, options)))

    async def handle_persistent_qa_approval_interaction(
        self,
        interaction: discord_button_qa_persistent_cases.PersistentQaInteraction,
        custom_id: str,
        *,
        approval_submitter: discord_button_qa_persistent_cases.Submitter,
    ) -> bool:
        handler = cast(
            discord_button_qa_persistent_cases.ApprovalInteractionHandler,
            getattr(self.module, "handle_persistent_approval_interaction"),
        )
        return await handler(
            cast(discord_button_qa_persistent_cases.PersistentQaInteraction, self._require_discord_interaction(interaction)),
            custom_id,
            approval_submitter=approval_submitter,
        )

    async def handle_persistent_qa_input_choice_interaction(
        self,
        interaction: discord_button_qa_persistent_cases.PersistentQaInteraction,
        custom_id: str,
        *,
        input_submitter: discord_button_qa_persistent_cases.Submitter,
    ) -> bool:
        handler = cast(
            discord_button_qa_persistent_cases.InputChoiceInteractionHandler,
            getattr(self.module, "handle_persistent_input_choice_interaction"),
        )
        return await handler(
            cast(discord_button_qa_persistent_cases.PersistentQaInteraction, self._require_discord_interaction(interaction)),
            custom_id,
            input_submitter=input_submitter,
        )

    def require_runtime_codex_bot(
        self,
        bot: discord_slash_commands.SlashCommandBot,
    ) -> discord_button_qa_lifecycle_cases.LifecycleQaBot:
        bot_type = cast(type, getattr(self.module, "CodexDiscordBot"))
        if isinstance(bot, bot_type):
            return cast(discord_button_qa_lifecycle_cases.LifecycleQaBot, bot)
        raise RuntimeSlashBotTypeError(bot)

    def make_button_qa_runtime(self) -> discord_bot_button_qa_runtime.BotButtonQaRuntime:
        return discord_bot_button_qa_runtime.BotButtonQaRuntime(
            discord_bot_button_qa_runtime.BotButtonQaRuntimeDeps(
                get_db_path=lambda: cast(Path, getattr(self.module, "MIRROR_DB_PATH")),
                get_mirrored_codex_thread_id=cast(
                    Callable[[int | None], str | None],
                    getattr(self.module, "get_mirrored_codex_thread_id"),
                ),
                make_busy_choice_payload=self.make_button_qa_busy_choice_payload,
                send_busy_choice_qa_message_tracked=self.send_busy_choice_qa_message_tracked,
                parse_busy_choice_custom_id=cast(
                    Callable[[str], tuple[str, str] | None],
                    getattr(self.module, "parse_busy_choice_custom_id"),
                ),
                is_button=cast(
                    Callable[[discord_bot_button_qa_runtime.ButtonQaViewChild], bool],
                    getattr(self.module, "is_discord_button_item"),
                ),
                handle_lifecycle_busy_choice=self.handle_lifecycle_qa_busy_choice_interaction,
                claim_busy_choice_record=cast(Callable[[str], bool], getattr(self.module, "claim_busy_choice_record")),
                get_busy_choice_record=cast(
                    Callable[
                        [str],
                        discord_button_qa_lifecycle_cases.BusyChoiceRecord | None,
                    ],
                    getattr(self.module, "get_busy_choice_record"),
                ),
                clear_stale_busy_choice_message_components=cast(
                    discord_button_qa_lifecycle_cases.StaleCleanupFunc,
                    getattr(self.module, "clear_stale_busy_choice_message_components"),
                ),
                handle_steer_busy_choice=self.handle_steer_qa_busy_choice_interaction,
                make_approval_view=self.make_persistent_qa_approval_view,
                make_input_choice_view=self.make_persistent_qa_input_choice_view,
                send_persistent_button_qa_message_tracked=self.send_persistent_button_qa_message_tracked,
                handle_persistent_approval_interaction=self.handle_persistent_qa_approval_interaction,
                handle_persistent_input_choice_interaction=self.handle_persistent_qa_input_choice_interaction,
                require_runtime_codex_bot=self.require_runtime_codex_bot,
                log_line=cast(Callable[[str], None], getattr(self.module, "log_line")),
            )
        )

    async def run_discord_button_qa(
        self,
        bot: discord_button_qa_lifecycle_cases.LifecycleQaBot,
        message: discord_button_qa_cases.ButtonQaMessage,
    ) -> str:
        return await self.make_button_qa_runtime().run_discord_button_qa(bot, message)

    async def run_prefix_discord_button_qa(
        self,
        bot: discord_button_qa_lifecycle_cases.LifecycleQaBot,
        message: discord_prefix_qa_command.MessageLike,
    ) -> str:
        return await self.make_button_qa_runtime().run_prefix_discord_button_qa(bot, message)

    async def run_runtime_discord_button_qa(
        self,
        bot: discord_slash_commands.SlashCommandBot,
        message: discord_slash_runtime_commands.RuntimeSlashSourceMessage,
    ) -> str:
        return await self.make_button_qa_runtime().run_runtime_discord_button_qa(bot, message)

    async def _send_message_tracked(
        self,
        channel: QaChannel,
        content: str,
        *,
        view: QaView | None,
        context: str,
    ) -> discord_button_qa_cases.SentMessageLike:
        require_messageable = cast(QaMessageableResolver, getattr(self.module, "require_discord_messageable"))
        sender = cast(QaSendMessageTracked, getattr(self.module, "send_message_tracked"))
        return await sender(require_messageable(channel), content, view=view, context=context)
