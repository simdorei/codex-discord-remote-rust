from __future__ import annotations

from dataclasses import dataclass
from types import ModuleType
from typing import cast

import codex_discord_approval_button_action as discord_approval_button_action
import codex_discord_approval_view as discord_approval_view
import codex_discord_busy_choice_view as discord_busy_choice_view
import codex_discord_input_choice_button_action as discord_input_choice_button_action
import codex_discord_input_choice_view as discord_input_choice_view


@dataclass(frozen=True, slots=True)
class BotComponentViewDepsRuntime:
    module: ModuleType

    def make_busy_choice_view_deps(self) -> discord_busy_choice_view.BusyChoiceViewDeps:
        module = self.module

        def claim_busy_choice_record(choice_id: str) -> bool:
            claim = cast(
                discord_busy_choice_view.BusyChoiceRecordClaimer,
                getattr(module, "claim_busy_choice_record"),
            )
            return claim(choice_id)

        return discord_busy_choice_view.BusyChoiceViewDeps(
            claim_busy_choice_record=claim_busy_choice_record,
            send_interaction_response=cast(
                discord_busy_choice_view.BusyInteractionResponseSender,
                getattr(module, "send_interaction_response_tracked"),
            ),
            send_direct_followup=cast(
                discord_busy_choice_view.BusyDirectFollowupSender,
                getattr(module, "send_direct_followup"),
            ),
            clear_interaction_message_components=cast(
                discord_busy_choice_view.BusyComponentClearer,
                getattr(module, "clear_interaction_message_components"),
            ),
            make_steer_action_deps=cast(
                discord_busy_choice_view.SteerActionDepsFactory,
                getattr(module, "_make_busy_choice_steer_action_deps"),
            ),
            make_queue_action_deps=cast(
                discord_busy_choice_view.QueueActionDepsFactory,
                getattr(module, "_make_busy_choice_queue_action_deps"),
            ),
            make_stop_action_deps=cast(
                discord_busy_choice_view.StopActionDepsFactory,
                getattr(module, "_make_busy_choice_stop_action_deps"),
            ),
            log=cast(discord_busy_choice_view.LogFunc, getattr(module, "log_line")),
        )

    def make_approval_button_action_deps(self) -> discord_approval_button_action.ApprovalButtonActionDeps:
        module = self.module
        return discord_approval_button_action.ApprovalButtonActionDeps(
            make_post_approval_watch_result=cast(
                discord_approval_button_action.ApprovalWatchMaker,
                getattr(module, "make_post_approval_watch_result"),
            ),
            submit_approval_reply=cast(
                discord_approval_button_action.ApprovalSubmitter,
                getattr(module, "submit_approval_reply"),
            ),
            send_followup_chunks=cast(
                discord_approval_button_action.FollowupChunkSender,
                getattr(module, "send_followup_chunks"),
            ),
            stream_post_approval_result=cast(
                discord_approval_button_action.PostApprovalResultStreamer,
                getattr(module, "stream_post_approval_result_for_interaction"),
            ),
            format_log_text_len=cast(
                discord_approval_button_action.LogTextLenFormatter,
                getattr(module, "format_log_text_len"),
            ),
            log=cast(discord_approval_button_action.LogFunc, getattr(module, "log_line")),
        )

    def make_approval_view_deps(self) -> discord_approval_view.ApprovalViewDeps:
        module = self.module
        interaction_gate = cast(ModuleType, getattr(module, "discord_interaction_gate"))
        traceback_module = cast(ModuleType, getattr(module, "traceback"))
        return discord_approval_view.ApprovalViewDeps(
            is_user_allowed=cast(
                discord_approval_view.AllowedUserChecker,
                getattr(interaction_gate, "is_discord_user_allowed"),
            ),
            send_interaction_response=cast(
                discord_approval_view.InteractionResponseSender,
                getattr(module, "send_interaction_response_tracked"),
            ),
            require_interaction_message=cast(
                discord_approval_view.InteractionMessageResolver,
                getattr(module, "require_interaction_message"),
            ),
            delivery_exceptions=cast(tuple[type[BaseException], ...], getattr(module, "DISCORD_DELIVERY_EXCEPTIONS")),
            format_exception=cast(discord_approval_view.ExceptionFormatter, getattr(traceback_module, "format_exc")),
            make_action_deps=self.make_approval_button_action_deps,
            log=cast(discord_approval_view.LogFunc, getattr(module, "log_line")),
        )

    def make_input_choice_button_action_deps(
        self,
    ) -> discord_input_choice_button_action.InputChoiceButtonActionDeps:
        module = self.module
        return discord_input_choice_button_action.InputChoiceButtonActionDeps(
            submit_input_reply=cast(
                discord_input_choice_button_action.InputSubmitter,
                getattr(module, "submit_input_reply"),
            ),
            send_followup_chunks=cast(
                discord_input_choice_button_action.FollowupChunkSender,
                getattr(module, "send_followup_chunks"),
            ),
            format_log_text_len=cast(
                discord_input_choice_button_action.LogTextLenFormatter,
                getattr(module, "format_log_text_len"),
            ),
            log=cast(discord_input_choice_button_action.LogFunc, getattr(module, "log_line")),
        )

    def make_input_choice_view_deps(self) -> discord_input_choice_view.InputChoiceViewDeps:
        module = self.module
        interaction_gate = cast(ModuleType, getattr(module, "discord_interaction_gate"))
        traceback_module = cast(ModuleType, getattr(module, "traceback"))
        return discord_input_choice_view.InputChoiceViewDeps(
            is_user_allowed=cast(
                discord_input_choice_view.AllowedUserChecker,
                getattr(interaction_gate, "is_discord_user_allowed"),
            ),
            send_interaction_response=cast(
                discord_input_choice_view.InteractionResponseSender,
                getattr(module, "send_interaction_response_tracked"),
            ),
            require_interaction_message=cast(
                discord_input_choice_view.InteractionMessageResolver,
                getattr(module, "require_interaction_message"),
            ),
            delivery_exceptions=cast(tuple[type[BaseException], ...], getattr(module, "DISCORD_DELIVERY_EXCEPTIONS")),
            format_exception=cast(discord_input_choice_view.ExceptionFormatter, getattr(traceback_module, "format_exc")),
            make_action_deps=self.make_input_choice_button_action_deps,
            log=cast(discord_input_choice_view.LogFunc, getattr(module, "log_line")),
        )
