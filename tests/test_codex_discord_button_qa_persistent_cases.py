from __future__ import annotations

import unittest
from collections.abc import Iterable
from dataclasses import dataclass

import codex_discord_button_qa_persistent_cases as persistent_cases


@dataclass(frozen=True)
class FakeButton:
    label: str
    custom_id: str


@dataclass(frozen=True)
class FakeNonButton:
    label: str = "not used"
    custom_id: str = "not-a-button"


@dataclass(frozen=True)
class FakeView:
    children: Iterable[persistent_cases.ViewChildLike]


@dataclass(frozen=True)
class FakeApprovalView:
    children: Iterable[persistent_cases.ViewChildLike]


@dataclass
class FakeResponse:
    deferred: bool = False


@dataclass
class FakeFollowup:
    messages: list[str]


@dataclass
class FakeInteraction:
    response: FakeResponse
    followup: FakeFollowup
    custom_id: str


@dataclass(frozen=True)
class SendEvent:
    channel: object
    content: str
    view: object | None
    context: str


@dataclass(frozen=True)
class InteractionEvent:
    bot: object
    channel: object
    message: object
    user: object
    custom_id: str


@dataclass(frozen=True)
class FakeBot:
    name: str = "bot"


@dataclass(frozen=True)
class FakeChannel:
    id: int = 222


@dataclass(frozen=True)
class FakeUser:
    id: int = 333


class PersistentButtonQaCaseTests(unittest.IsolatedAsyncioTestCase):
    def make_deps(
        self,
        *,
        approval_children: Iterable[persistent_cases.ViewChildLike] | None = None,
        approval_followups: list[str] | None = None,
        approval_defer: bool = True,
        approval_handler_result: bool = True,
        input_children: Iterable[persistent_cases.ViewChildLike] | None = None,
        input_followups: list[str] | None = None,
        input_defer: bool = True,
        input_handler_result: bool = True,
        approval_handler_submissions: list[tuple[str, str]] | None = None,
        input_handler_submissions: list[tuple[str, str]] | None = None,
    ) -> tuple[persistent_cases.PersistentButtonQaCaseDeps, list[SendEvent], list[InteractionEvent]]:
        sends: list[SendEvent] = []
        interactions: list[InteractionEvent] = []

        async def send_message_tracked(
            channel: object,
            content: str,
            *,
            view: object | None = None,
            context: str = "send_message_tracked",
        ) -> object:
            sends.append(SendEvent(channel, content, view, context))
            return SendEvent(channel, content, view, context)

        def make_interaction(
            *,
            bot: object,
            channel: object,
            message: object,
            user: object,
            custom_id: str,
        ) -> FakeInteraction:
            interactions.append(InteractionEvent(bot, channel, message, user, custom_id))
            return FakeInteraction(
                response=FakeResponse(),
                followup=FakeFollowup(messages=[]),
                custom_id=custom_id,
            )

        async def handle_approval(
            interaction: persistent_cases.PersistentQaInteraction,
            custom_id: str,
            *,
            approval_submitter: persistent_cases.Submitter,
        ) -> bool:
            if not approval_handler_result:
                return False
            parts = custom_id.split(":")
            target_thread_id = parts[1]
            answer = parts[2]
            if approval_handler_submissions is not None:
                approval_handler_submissions.append((target_thread_id, answer))
            _exit_code, _output = approval_submitter(target_thread_id, answer)
            setattr(interaction.response, "deferred", approval_defer)
            messages = approval_followups or ["Approval submitted\n\napproved"]
            interaction.followup.messages.extend(messages)
            return True

        async def handle_input(
            interaction: persistent_cases.PersistentQaInteraction,
            custom_id: str,
            *,
            input_submitter: persistent_cases.Submitter,
        ) -> bool:
            if not input_handler_result:
                return False
            parts = custom_id.split(":")
            target_thread_id = parts[1]
            value = parts[2]
            if input_handler_submissions is not None:
                input_handler_submissions.append((target_thread_id, value))
            _exit_code, _output = input_submitter(target_thread_id, value)
            setattr(interaction.response, "deferred", input_defer)
            messages = input_followups or ["Input submitted\n\nanswered"]
            interaction.followup.messages.extend(messages)
            return True

        deps = persistent_cases.PersistentButtonQaCaseDeps(
            make_approval_view=lambda target_thread_id: FakeApprovalView(
                approval_children
                if approval_children is not None
                else [
                    FakeButton("Approve session", f"approval:{target_thread_id}:2"),
                    FakeNonButton(),
                ],
            ),
            make_input_choice_view=lambda target_thread_id, options: FakeView(
                input_children
                if input_children is not None
                else [FakeButton(label, f"input:{target_thread_id}:{value}") for value, label in options],
            ),
            make_interaction=make_interaction,
            send_message_tracked=send_message_tracked,
            handle_persistent_approval_interaction=handle_approval,
            handle_persistent_input_choice_interaction=handle_input,
            is_button=lambda item: isinstance(item, FakeButton),
        )
        return deps, sends, interactions

    async def test_runs_persistent_approval_and_input_cases(self) -> None:
        approval_handler_submissions: list[tuple[str, str]] = []
        input_handler_submissions: list[tuple[str, str]] = []
        deps, sends, _interactions = self.make_deps(
            approval_handler_submissions=approval_handler_submissions,
            input_handler_submissions=input_handler_submissions,
        )

        lines = await persistent_cases.run_persistent_button_qa_cases(
            bot=FakeBot(),
            channel=FakeChannel(),
            user=FakeUser(),
            deps=deps,
        )

        self.assertEqual(lines, ["approval_persistent: ok", "input_choice_persistent: ok"])
        self.assertEqual(approval_handler_submissions, [("qa-thread", "2")])
        self.assertEqual(input_handler_submissions, [("qa-thread", "choice-1")])
        self.assertEqual(sends[0].content, "QA approval persistent smoke")
        self.assertEqual(sends[0].context, "button_qa_approval")
        self.assertEqual(sends[1].content, "QA input persistent smoke")
        self.assertEqual(sends[1].context, "button_qa_input")

    async def test_reports_failed_lines_without_substituting_fallbacks(self) -> None:
        deps, _sends, _interactions = self.make_deps(approval_followups=["wrong"])

        lines = await persistent_cases.run_persistent_button_qa_cases(
            bot=object(),
            channel=object(),
            user=object(),
            deps=deps,
        )

        self.assertEqual(lines, ["approval_persistent: failed", "input_choice_persistent: ok"])

    async def test_preserves_missing_button_key_errors_and_non_button_filtering(self) -> None:
        deps, _sends, _interactions = self.make_deps(approval_children=[FakeNonButton()])

        with self.assertRaises(KeyError):
            _ = await persistent_cases.run_persistent_button_qa_cases(
                bot=object(),
                channel=object(),
                user=object(),
                deps=deps,
            )

        deps, _sends, _interactions = self.make_deps(input_children=[FakeNonButton()])

        with self.assertRaises(KeyError):
            _ = await persistent_cases.run_persistent_button_qa_cases(
                bot=object(),
                channel=object(),
                user=object(),
                deps=deps,
            )

    async def test_false_handlers_and_missing_defer_return_failed_without_submitter_side_effects(self) -> None:
        approval_handler_submissions: list[tuple[str, str]] = []
        deps, _sends, _interactions = self.make_deps(
            approval_handler_result=False,
            approval_handler_submissions=approval_handler_submissions,
        )

        lines = await persistent_cases.run_persistent_button_qa_cases(
            bot=object(),
            channel=object(),
            user=object(),
            deps=deps,
        )

        self.assertEqual(lines, ["approval_persistent: failed", "input_choice_persistent: ok"])
        self.assertEqual(approval_handler_submissions, [])

        deps, _sends, _interactions = self.make_deps(input_defer=False)

        lines = await persistent_cases.run_persistent_button_qa_cases(
            bot=object(),
            channel=object(),
            user=object(),
            deps=deps,
        )

        self.assertEqual(lines, ["approval_persistent: ok", "input_choice_persistent: failed"])


if __name__ == "__main__":
    _ = unittest.main()
