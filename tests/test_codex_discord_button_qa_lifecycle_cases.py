from __future__ import annotations

import unittest
from dataclasses import dataclass, field

import codex_discord_button_qa_lifecycle_cases as lifecycle_cases
from codex_discord_components import ComponentRowLike


@dataclass(frozen=True)
class FakeMessage:
    content: str
    components: tuple[ComponentRowLike, ...] | None = None


@dataclass(frozen=True)
class FakeResponse:
    messages: list[str] = field(default_factory=list)


@dataclass(frozen=True)
class FakeInteraction:
    response: FakeResponse
    custom_id: str


@dataclass(frozen=True)
class CaseEvent:
    prompt: str
    choice_id: str


@dataclass(frozen=True)
class InteractionEvent:
    message: object
    custom_id: str


class BusyChoiceLifecycleQaCaseTests(unittest.IsolatedAsyncioTestCase):
    def make_deps(
        self,
        *,
        case_custom_ids: dict[str, dict[str, str]] | None = None,
        handler_messages: dict[str, list[str]] | None = None,
        handler_results: dict[str, bool] | None = None,
        cleanup_result: bool = True,
        stale_record_after_cleanup: bool = False,
    ) -> tuple[lifecycle_cases.BusyChoiceLifecycleQaCaseDeps, list[CaseEvent], list[InteractionEvent]]:
        records: dict[str, bool] = {}
        cases: list[CaseEvent] = []
        interactions: list[InteractionEvent] = []
        custom_ids_by_prompt = case_custom_ids or {}
        messages_by_action = handler_messages or {}
        results_by_action = handler_results or {}

        async def send_case_button(prompt: str) -> lifecycle_cases.SendCaseButtonResult:
            choice_id = _choice_id(prompt)
            records[choice_id] = True
            cases.append(CaseEvent(prompt, choice_id))
            custom_ids = custom_ids_by_prompt.get(
                prompt,
                {
                    "Ignore": f"ignore:{choice_id}",
                    "Queue next": f"queue:{choice_id}",
                    "Steer now": f"steer:{choice_id}",
                },
            )
            return FakeMessage(prompt), custom_ids, choice_id

        def make_interaction(
            *,
            bot: object,
            channel: object,
            message: object,
            user: object,
            custom_id: str,
        ) -> FakeInteraction:
            _ = (bot, channel, user)
            interactions.append(InteractionEvent(message, custom_id))
            return FakeInteraction(response=FakeResponse(), custom_id=custom_id)

        async def handle_persistent_busy_choice_interaction(
            interaction: lifecycle_cases.BusyChoiceQaInteraction,
            custom_id: str,
        ) -> bool:
            action, choice_id = custom_id.split(":", 1)
            match action:
                case "ignore":
                    records[choice_id] = False
                    default_message = "Ignored."
                case "queue" | "steer":
                    default_message = lifecycle_cases.STALE_BUTTON_MESSAGE
                case unreachable:
                    raise AssertionError(f"unexpected action: {unreachable}")
            interaction.response.messages.extend(messages_by_action.get(action, [default_message]))
            return results_by_action.get(action, True)

        def claim_busy_choice_record(choice_id: str) -> bool:
            records[choice_id] = False
            return True

        def get_busy_choice_record(choice_id: str) -> object | None:
            if stale_record_after_cleanup and choice_id == "stale-cleanup":
                return {"choice_id": choice_id}
            return {"choice_id": choice_id} if records.get(choice_id, False) else None

        def delete_busy_choice_record(choice_id: str) -> None:
            records[choice_id] = False

        async def clear_stale_busy_choice_message_components(message: object) -> bool:
            _ = message
            return cleanup_result

        deps = lifecycle_cases.BusyChoiceLifecycleQaCaseDeps(
            send_case_button=send_case_button,
            make_interaction=make_interaction,
            handle_persistent_busy_choice_interaction=handle_persistent_busy_choice_interaction,
            claim_busy_choice_record=claim_busy_choice_record,
            get_busy_choice_record=get_busy_choice_record,
            delete_busy_choice_record=delete_busy_choice_record,
            clear_stale_busy_choice_message_components=clear_stale_busy_choice_message_components,
        )
        return deps, cases, interactions

    async def test_runs_busy_choice_lifecycle_cases(self) -> None:
        deps, cases, interactions = self.make_deps()

        lines = await lifecycle_cases.run_busy_choice_lifecycle_qa_cases(
            bot=object(),
            channel=object(),
            user=object(),
            deps=deps,
        )

        self.assertEqual(
            lines,
            ["ignore: ok", "claimed_record: ok", "missing_record: ok", "stale_cleanup: ok"],
        )
        self.assertEqual(
            [case.prompt for case in cases],
            [
                "QA button ignore smoke",
                "QA button claimed-record smoke",
                "QA button missing-record smoke",
                "QA button stale cleanup smoke",
            ],
        )
        self.assertEqual([event.custom_id for event in interactions], ["ignore:ignore", "queue:claimed", "steer:missing"])

    async def test_failed_predicates_return_failed_lines(self) -> None:
        deps, _cases, _interactions = self.make_deps(handler_messages={"ignore": ["wrong"]})

        lines = await lifecycle_cases.run_busy_choice_lifecycle_qa_cases(
            bot=object(),
            channel=object(),
            user=object(),
            deps=deps,
        )

        self.assertEqual(lines[0], "ignore: failed")

        deps, _cases, _interactions = self.make_deps(handler_results={"queue": False})
        lines = await lifecycle_cases.run_busy_choice_lifecycle_qa_cases(
            bot=object(),
            channel=object(),
            user=object(),
            deps=deps,
        )
        self.assertEqual(lines[1], "claimed_record: failed")

        deps, _cases, _interactions = self.make_deps(handler_messages={"steer": ["wrong"]})
        lines = await lifecycle_cases.run_busy_choice_lifecycle_qa_cases(
            bot=object(),
            channel=object(),
            user=object(),
            deps=deps,
        )
        self.assertEqual(lines[2], "missing_record: failed")

        deps, _cases, _interactions = self.make_deps(cleanup_result=False, stale_record_after_cleanup=True)
        lines = await lifecycle_cases.run_busy_choice_lifecycle_qa_cases(
            bot=object(),
            channel=object(),
            user=object(),
            deps=deps,
        )
        self.assertEqual(lines[3], "stale_cleanup: failed")

    async def test_missing_required_custom_ids_raise_key_error(self) -> None:
        deps, _cases, _interactions = self.make_deps(
            case_custom_ids={"QA button ignore smoke": {"Queue next": "queue:ignore"}},
        )

        with self.assertRaises(KeyError):
            _ = await lifecycle_cases.run_busy_choice_lifecycle_qa_cases(
                bot=object(),
                channel=object(),
                user=object(),
                deps=deps,
            )


def _choice_id(prompt: str) -> str:
    match prompt:
        case "QA button ignore smoke":
            return "ignore"
        case "QA button claimed-record smoke":
            return "claimed"
        case "QA button missing-record smoke":
            return "missing"
        case "QA button stale cleanup smoke":
            return "stale-cleanup"
        case unreachable:
            raise AssertionError(f"unexpected prompt: {unreachable}")


if __name__ == "__main__":
    _ = unittest.main()
