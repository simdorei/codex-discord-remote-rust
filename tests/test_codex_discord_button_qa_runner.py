from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import TypeAlias, cast
import unittest

import codex_discord_button_qa_cases as button_qa_cases
import codex_discord_button_qa_lifecycle_cases as lifecycle_cases
import codex_discord_button_qa_persistent_cases as persistent_cases
import codex_discord_button_qa_steer_case as steer_case
import codex_discord_button_qa_runner as runner

FakeSentMessage: TypeAlias = lifecycle_cases.LifecycleQaMessage | steer_case.SteerQaMessage
FakeSendCaseResult: TypeAlias = tuple[FakeSentMessage, dict[str, str], str]


@dataclass(frozen=True, slots=True)
class FakeAuthor:
    id: int = 333
    bot: bool = False


@dataclass(frozen=True, slots=True)
class FakeChannel:
    id: int = 222


@dataclass(frozen=True, slots=True)
class FakeMessage:
    channel: FakeChannel
    author: FakeAuthor


@dataclass(frozen=True, slots=True)
class FakeQaCase:
    sent_message: str
    custom_ids: dict[str, str]
    choice_id: str


@dataclass(frozen=True, slots=True)
class FakeSendCaseDeps:
    send_case_button: Callable[[str], Awaitable[FakeSendCaseResult]]


@dataclass(frozen=True, slots=True)
class FakePersistentDeps:
    pass


class ButtonQaRunnerTests(unittest.IsolatedAsyncioTestCase):
    async def test_runs_button_qa_cases_and_reports_failed_result(self) -> None:
        events: list[str] = []
        logs: list[str] = []

        async def fake_send_busy_choice_qa_case(
            message: FakeMessage,
            channel: FakeChannel,
            prompt: str,
            *,
            deps: button_qa_cases.BusyChoiceQaCaseDeps,
        ) -> FakeQaCase:
            events.append(f"send:{message.channel.id}:{channel.id}:{prompt}:{deps}")
            return FakeQaCase("sent-message", {"Ignore": "ignore-id", "Steer now": "steer-id"}, "choice-1")

        async def fake_lifecycle_cases(
            *,
            bot: str,
            channel: FakeChannel,
            user: FakeAuthor,
            deps: FakeSendCaseDeps,
        ) -> list[str]:
            _ = (bot, channel, user)
            sent_message, _custom_ids, choice_id = await deps.send_case_button("lifecycle")
            events.append(f"lifecycle:{sent_message}:{choice_id}")
            return ["ignore: ok", "claimed_record: ok"]

        async def fake_steer_case(
            *,
            bot: str,
            channel: FakeChannel,
            user: FakeAuthor,
            deps: FakeSendCaseDeps,
        ) -> str:
            _ = (bot, channel, user)
            sent_message, _custom_ids, choice_id = await deps.send_case_button("steer")
            events.append(f"steer:{sent_message}:{choice_id}")
            return "steer_success: failed"

        async def fake_persistent_cases(
            *,
            bot: str,
            channel: FakeChannel,
            user: FakeAuthor,
            deps: FakePersistentDeps,
        ) -> list[str]:
            _ = (bot, channel, user, deps)
            events.append("persistent")
            return ["approval_persistent: ok"]

        original_send = button_qa_cases.send_busy_choice_qa_case
        original_lifecycle = lifecycle_cases.run_busy_choice_lifecycle_qa_cases
        original_steer = steer_case.run_busy_choice_steer_success_qa_case
        original_persistent = persistent_cases.run_persistent_button_qa_cases
        try:
            button_qa_cases.send_busy_choice_qa_case = fake_send_busy_choice_qa_case
            lifecycle_cases.run_busy_choice_lifecycle_qa_cases = fake_lifecycle_cases
            steer_case.run_busy_choice_steer_success_qa_case = fake_steer_case
            persistent_cases.run_persistent_button_qa_cases = fake_persistent_cases

            output = await runner.run_discord_button_qa(
                "bot",
                cast(button_qa_cases.ButtonQaMessage, cast(object, FakeMessage(FakeChannel(), FakeAuthor()))),
                deps=runner.ButtonQaRunnerDeps(
                    make_case_deps=lambda: cast(button_qa_cases.BusyChoiceQaCaseDeps, cast(object, "case-deps")),
                    make_lifecycle_case_deps=lambda send_case_button: cast(
                        lifecycle_cases.BusyChoiceLifecycleQaCaseDeps,
                        cast(object, FakeSendCaseDeps(send_case_button)),
                    ),
                    make_steer_case_deps=lambda send_case_button: cast(
                        steer_case.BusyChoiceSteerQaCaseDeps,
                        cast(object, FakeSendCaseDeps(send_case_button)),
                    ),
                    make_persistent_case_deps=lambda: cast(
                        persistent_cases.PersistentButtonQaCaseDeps,
                        cast(object, FakePersistentDeps()),
                    ),
                    log_line=logs.append,
                ),
            )
        finally:
            button_qa_cases.send_busy_choice_qa_case = original_send
            lifecycle_cases.run_busy_choice_lifecycle_qa_cases = original_lifecycle
            steer_case.run_busy_choice_steer_success_qa_case = original_steer
            persistent_cases.run_persistent_button_qa_cases = original_persistent

        self.assertEqual(
            output.splitlines(),
            [
                "Discord button QA",
                "ignore: ok",
                "claimed_record: ok",
                "steer_success: failed",
                "approval_persistent: ok",
                "result: failed",
            ],
        )
        self.assertEqual(
            events,
            [
                "send:222:222:lifecycle:case-deps",
                "lifecycle:sent-message:choice-1",
                "send:222:222:steer:case-deps",
                "steer:sent-message:choice-1",
                "persistent",
            ],
        )
        self.assertEqual(
            logs,
            ["button_qa_start channel=222 user=333", "button_qa_done channel=222 user=333 result=failed"],
        )


if __name__ == "__main__":
    _ = unittest.main()
