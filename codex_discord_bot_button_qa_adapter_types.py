from __future__ import annotations

from collections.abc import Awaitable, Callable
from typing import Protocol, TypeAlias

import codex_discord_button_qa_cases as discord_button_qa_cases
import codex_discord_button_qa_lifecycle_cases as discord_button_qa_lifecycle_cases
import codex_discord_button_qa_persistent_cases as discord_button_qa_persistent_cases
import codex_discord_button_qa_steer_case as discord_button_qa_steer_case
import codex_discord_slash_commands as discord_slash_commands

QaChannel: TypeAlias = (
    discord_button_qa_cases.ButtonQaChannel | discord_button_qa_persistent_cases.PersistentQaChannel
)
QaView: TypeAlias = discord_button_qa_cases.ViewLike | discord_button_qa_persistent_cases.ViewLike
QaInteraction: TypeAlias = (
    discord_button_qa_lifecycle_cases.BusyChoiceQaInteraction
    | discord_button_qa_steer_case.SteerQaInteraction
    | discord_button_qa_persistent_cases.PersistentQaInteraction
)
QaMessageableResolver: TypeAlias = Callable[[QaChannel], QaChannel]
QaInteractionResolver: TypeAlias = Callable[[QaInteraction], QaInteraction]


class QaSendMessageTracked(Protocol):
    def __call__(
        self,
        target: QaChannel,
        content: str,
        *,
        view: QaView | None = None,
        context: str = "send_message_tracked",
    ) -> Awaitable[discord_button_qa_cases.SentMessageLike]: ...


class RuntimeSlashBotTypeError(TypeError):
    def __init__(self, bot: discord_slash_commands.SlashCommandBot) -> None:
        super().__init__(f"Expected CodexDiscordBot, got {type(bot).__name__}")
