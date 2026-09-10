from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from typing import cast

import codex_discord_button_qa_cases as discord_button_qa_cases
import codex_discord_button_qa_lifecycle_cases as discord_button_qa_lifecycle_cases
import codex_discord_button_qa_persistent_cases as discord_button_qa_persistent_cases
import codex_discord_button_qa_steer_case as discord_button_qa_steer_case


@dataclass(frozen=True, slots=True)
class ButtonQaRunnerDeps:
    make_case_deps: Callable[[], discord_button_qa_cases.BusyChoiceQaCaseDeps]
    make_lifecycle_case_deps: Callable[
        [discord_button_qa_lifecycle_cases.SendCaseButtonFunc],
        discord_button_qa_lifecycle_cases.BusyChoiceLifecycleQaCaseDeps,
    ]
    make_steer_case_deps: Callable[
        [discord_button_qa_steer_case.SendCaseButtonFunc],
        discord_button_qa_steer_case.BusyChoiceSteerQaCaseDeps,
    ]
    make_persistent_case_deps: Callable[[], discord_button_qa_persistent_cases.PersistentButtonQaCaseDeps]
    log_line: Callable[[str], None]


async def run_discord_button_qa(
    bot: discord_button_qa_lifecycle_cases.LifecycleQaBot,
    message: discord_button_qa_cases.ButtonQaMessage,
    *,
    deps: ButtonQaRunnerDeps,
) -> str:
    channel = message.channel
    user = message.author
    button_channel = cast(discord_button_qa_cases.ButtonQaChannel, cast(object, channel))
    lifecycle_channel = cast(discord_button_qa_lifecycle_cases.LifecycleQaChannel, cast(object, channel))
    steer_channel = cast(discord_button_qa_steer_case.SteerQaChannel, cast(object, channel))
    persistent_channel = cast(discord_button_qa_persistent_cases.PersistentQaChannel, cast(object, channel))
    lines = ["Discord button QA"]

    async def send_case_button(
        prompt: str,
    ) -> discord_button_qa_lifecycle_cases.SendCaseButtonResult:
        qa_case = await discord_button_qa_cases.send_busy_choice_qa_case(
            message,
            button_channel,
            prompt,
            deps=deps.make_case_deps(),
        )
        return qa_case.sent_message, qa_case.custom_ids, qa_case.choice_id

    deps.log_line(f"button_qa_start channel={getattr(channel, 'id', '-')} user={getattr(user, 'id', '-')}")

    lines.extend(
        await discord_button_qa_lifecycle_cases.run_busy_choice_lifecycle_qa_cases(
            bot=bot,
            channel=lifecycle_channel,
            user=user,
            deps=deps.make_lifecycle_case_deps(send_case_button),
        )
    )
    lines.append(
        await discord_button_qa_steer_case.run_busy_choice_steer_success_qa_case(
            bot=bot,
            channel=steer_channel,
            user=user,
            deps=deps.make_steer_case_deps(cast(discord_button_qa_steer_case.SendCaseButtonFunc, send_case_button)),
        )
    )
    lines.extend(
        await discord_button_qa_persistent_cases.run_persistent_button_qa_cases(
            bot=bot,
            channel=persistent_channel,
            user=user,
            deps=deps.make_persistent_case_deps(),
        )
    )

    passed = all(line.endswith(": ok") for line in lines[1:])
    lines.append(f"result: {'ok' if passed else 'failed'}")
    deps.log_line(
        f"button_qa_done channel={getattr(channel, 'id', '-')}"
        + f" user={getattr(user, 'id', '-')} result={'ok' if passed else 'failed'}"
    )
    return "\n".join(lines)
