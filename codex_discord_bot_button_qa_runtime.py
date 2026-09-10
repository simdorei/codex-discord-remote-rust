from __future__ import annotations

import sqlite3
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from typing import cast

import codex_discord_button_qa_cases as discord_button_qa_cases
from codex_discord_store_connection import connect_store
import codex_discord_button_qa_interactions as discord_button_qa_interactions
import codex_discord_button_qa_lifecycle_cases as discord_button_qa_lifecycle_cases
import codex_discord_button_qa_persistent_cases as discord_button_qa_persistent_cases
import codex_discord_button_qa_runner as discord_button_qa_runner
import codex_discord_button_qa_steer_case as discord_button_qa_steer_case
import codex_discord_busy_choice_source_message as discord_busy_choice_source_message
import codex_discord_prefix_qa_command as discord_prefix_qa_command
import codex_discord_slash_commands as discord_slash_commands
import codex_discord_slash_runtime_commands as discord_slash_runtime_commands
import codex_discord_steering as discord_steering

ButtonQaViewChild = (
    discord_button_qa_cases.ViewChildLike | discord_button_qa_persistent_cases.ViewChildLike
)


@dataclass(frozen=True, slots=True)
class _RuntimeButtonQaAuthor:
    id: int
    bot: bool = False


@dataclass(frozen=True, slots=True)
class _RuntimeButtonQaMessage:
    author: _RuntimeButtonQaAuthor
    channel: discord_slash_runtime_commands.RuntimeSlashChannel


@dataclass(frozen=True, slots=True)
class BotButtonQaRuntimeDeps:
    get_db_path: Callable[[], Path]
    get_mirrored_codex_thread_id: Callable[[int | None], str | None]
    make_busy_choice_payload: discord_button_qa_cases.MakeBusyChoicePayloadFunc
    send_busy_choice_qa_message_tracked: discord_button_qa_cases.SendMessageTrackedFunc
    parse_busy_choice_custom_id: Callable[[str], tuple[str, str] | None]
    is_button: Callable[[ButtonQaViewChild], bool]
    handle_lifecycle_busy_choice: discord_button_qa_lifecycle_cases.BusyChoiceInteractionHandler
    claim_busy_choice_record: Callable[[str], bool]
    get_busy_choice_record: Callable[
        [str],
        discord_button_qa_lifecycle_cases.BusyChoiceRecord | None,
    ]
    clear_stale_busy_choice_message_components: discord_button_qa_lifecycle_cases.StaleCleanupFunc
    handle_steer_busy_choice: discord_button_qa_steer_case.BusyChoiceSteerHandler
    make_approval_view: Callable[[str], discord_button_qa_persistent_cases.ViewLike]
    make_input_choice_view: discord_button_qa_persistent_cases.MakeInputChoiceViewFunc
    send_persistent_button_qa_message_tracked: (
        discord_button_qa_persistent_cases.SendMessageTrackedFunc
    )
    handle_persistent_approval_interaction: (
        discord_button_qa_persistent_cases.ApprovalInteractionHandler
    )
    handle_persistent_input_choice_interaction: (
        discord_button_qa_persistent_cases.InputChoiceInteractionHandler
    )
    require_runtime_codex_bot: Callable[
        [discord_slash_commands.SlashCommandBot],
        discord_button_qa_lifecycle_cases.LifecycleQaBot,
    ]
    log_line: Callable[[str], None]


@dataclass(frozen=True, slots=True)
class BotButtonQaRuntime:
    deps: BotButtonQaRuntimeDeps

    async def run_discord_button_qa(
        self,
        bot: discord_button_qa_lifecycle_cases.LifecycleQaBot,
        message: discord_button_qa_cases.ButtonQaMessage,
    ) -> str:
        return await discord_button_qa_runner.run_discord_button_qa(
            bot,
            message,
            deps=discord_button_qa_runner.ButtonQaRunnerDeps(
                make_case_deps=self.make_case_deps,
                make_lifecycle_case_deps=self.make_lifecycle_case_deps,
                make_steer_case_deps=self.make_steer_case_deps,
                make_persistent_case_deps=self.make_persistent_case_deps,
                log_line=self.deps.log_line,
            ),
        )

    async def run_prefix_discord_button_qa(
        self,
        bot: discord_button_qa_lifecycle_cases.LifecycleQaBot,
        message: discord_prefix_qa_command.MessageLike,
    ) -> str:
        return await self.run_discord_button_qa(
            bot,
            discord_busy_choice_source_message.make_runtime_busy_choice_source_message(
                cast(
                    discord_busy_choice_source_message.RuntimeBusyChoiceMessageLike,
                    cast(object, message),
                ),
            ),
        )

    async def run_runtime_discord_button_qa(
        self,
        bot: discord_slash_commands.SlashCommandBot,
        message: discord_slash_runtime_commands.RuntimeSlashSourceMessage,
    ) -> str:
        return await self.run_discord_button_qa(
            self.deps.require_runtime_codex_bot(bot),
            discord_busy_choice_source_message.make_runtime_busy_choice_source_message(
                cast(
                    discord_busy_choice_source_message.RuntimeBusyChoiceMessageLike,
                    cast(
                        object,
                        _RuntimeButtonQaMessage(
                            author=_RuntimeButtonQaAuthor(id=message.author.id),
                            channel=message.channel,
                        ),
                    ),
                ),
            ),
        )

    def make_case_deps(self) -> discord_button_qa_cases.BusyChoiceQaCaseDeps:
        return discord_button_qa_cases.BusyChoiceQaCaseDeps(
            get_mirrored_codex_thread_id=self.deps.get_mirrored_codex_thread_id,
            make_busy_choice_payload=self.deps.make_busy_choice_payload,
            send_message_tracked=self.deps.send_busy_choice_qa_message_tracked,
            parse_busy_choice_custom_id=self.deps.parse_busy_choice_custom_id,
            is_button=self.deps.is_button,
        )

    def make_lifecycle_case_deps(
        self,
        send_case_button: discord_button_qa_lifecycle_cases.SendCaseButtonFunc,
    ) -> discord_button_qa_lifecycle_cases.BusyChoiceLifecycleQaCaseDeps:
        return discord_button_qa_lifecycle_cases.BusyChoiceLifecycleQaCaseDeps(
            send_case_button=send_case_button,
            make_interaction=discord_button_qa_interactions.make_lifecycle_qa_interaction,
            handle_persistent_busy_choice_interaction=self.deps.handle_lifecycle_busy_choice,
            claim_busy_choice_record=self.deps.claim_busy_choice_record,
            get_busy_choice_record=self.deps.get_busy_choice_record,
            delete_busy_choice_record=self.delete_busy_choice_record,
            clear_stale_busy_choice_message_components=(
                self.deps.clear_stale_busy_choice_message_components
            ),
        )

    def make_steer_case_deps(
        self,
        send_case_button: discord_button_qa_steer_case.SendCaseButtonFunc,
    ) -> discord_button_qa_steer_case.BusyChoiceSteerQaCaseDeps:
        return discord_button_qa_steer_case.BusyChoiceSteerQaCaseDeps(
            send_case_button=send_case_button,
            make_interaction=discord_button_qa_interactions.make_steer_qa_interaction,
            handle_persistent_busy_choice_interaction=self.deps.handle_steer_busy_choice,
            delete_busy_choice_record=self.delete_busy_choice_record,
            get_mirrored_codex_thread_id=self.deps.get_mirrored_codex_thread_id,
            make_steering_prompt_result=self.make_steering_prompt_result,
        )

    def make_persistent_case_deps(
        self,
    ) -> discord_button_qa_persistent_cases.PersistentButtonQaCaseDeps:
        return discord_button_qa_persistent_cases.PersistentButtonQaCaseDeps(
            make_approval_view=self.deps.make_approval_view,
            make_input_choice_view=self.deps.make_input_choice_view,
            make_interaction=discord_button_qa_interactions.make_persistent_qa_interaction,
            send_message_tracked=self.deps.send_persistent_button_qa_message_tracked,
            handle_persistent_approval_interaction=(
                self.deps.handle_persistent_approval_interaction
            ),
            handle_persistent_input_choice_interaction=(
                self.deps.handle_persistent_input_choice_interaction
            ),
            is_button=self.deps.is_button,
        )

    def delete_busy_choice_record(self, choice_id: str) -> None:
        with connect_store(self.deps.get_db_path()) as conn:
            _ = conn.execute("DELETE FROM busy_choices WHERE choice_id = ?", (choice_id,))

    def make_steering_prompt_result(
        self,
        target_thread_id: str | None,
    ) -> discord_steering.SteeringPromptResult:
        return discord_steering.SteeringPromptResult(
            0,
            "[qa_delivery_verified]",
            target_thread_id=target_thread_id,
            target_ref=target_thread_id or "-",
            session_path="qa-session.jsonl",
            start_offset=0,
        )
