from __future__ import annotations

from collections.abc import Callable
from types import ModuleType
from typing import TypeAlias, cast

import codex_discord_interaction_gate as discord_interaction_gate
import codex_discord_resume_view as discord_resume_view

PromptChannel: TypeAlias = object


async def send_resume_failure(
    module: ModuleType,
    channel: PromptChannel,
    content: str,
    target_thread_id: str,
) -> None:
    view = discord_resume_view.ResumeView(
        target_thread_id,
        deps=discord_resume_view.ResumeViewDeps(
            recover_resident_thread_for_request=cast(
                discord_resume_view.RecoverResidentThreadFunc,
                getattr(module, "recover_resident_thread_for_request"),
            ),
            is_user_allowed=discord_interaction_gate.is_discord_user_allowed,
            send_interaction_response=cast(
                discord_resume_view.InteractionResponseSender,
                getattr(module, "send_interaction_response_tracked"),
            ),
            send_direct_followup=cast(
                discord_resume_view.DirectFollowupSender,
                getattr(module, "send_direct_followup"),
            ),
            log=cast(Callable[[str], None], getattr(module, "log_line")),
        ),
    )
    _ = await cast(
        discord_resume_view.ResumeFailureMessageSender[PromptChannel],
        getattr(module, "send_message_tracked"),
    )(
        channel,
        content,
        view=view,
        context="resume_failure",
    )
