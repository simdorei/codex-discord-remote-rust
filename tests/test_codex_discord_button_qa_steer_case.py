from __future__ import annotations

import unittest
from collections.abc import Callable
from dataclasses import dataclass, field

import codex_discord_button_qa_steer_case as steer_case
import codex_discord_steering as steering


@dataclass(frozen=True)
class FakeMessage:
    content: str


@dataclass(frozen=True)
class FakeChannel:
    id: int = 222


@dataclass
class FakeResponse:
    deferred: bool = False
    defer_kwargs: list[steer_case.DeferKwargs] = field(default_factory=list)

    async def defer(self, *, thinking: bool, ephemeral: bool) -> None:
        self.deferred = True
        self.defer_kwargs.append({"thinking": thinking, "ephemeral": ephemeral})


@dataclass
class FakeFollowup:
    messages: list[str] = field(default_factory=list)


@dataclass(frozen=True)
class FakeUser:
    id: int = 123


@dataclass
class FakeInteraction:
    response: FakeResponse
    followup: FakeFollowup
    custom_id: str
    user: steer_case.SteerQaUser
    channel_id: int | None = 222


@dataclass(frozen=True)
class CaseEvent:
    prompt: str
    choice_id: str


@dataclass(frozen=True)
class InteractionEvent:
    message: object
    custom_id: str


class BusyChoiceSteerQaCaseTests(unittest.IsolatedAsyncioTestCase):
    def make_deps(
        self,
        *,
        custom_ids: dict[str, str] | None = None,
        handler_result: bool = True,
        defer_response: bool = True,
        ephemeral: bool = True,
        followup_message: str = "Steering sent to qa-thread",
        result_target_override: str | None = None,
    ) -> tuple[steer_case.BusyChoiceSteerQaCaseDeps, list[CaseEvent], list[InteractionEvent], list[str]]:
        cases: list[CaseEvent] = []
        interactions: list[InteractionEvent] = []
        deleted_choice_ids: list[str] = []

        async def send_case_button(prompt: str) -> tuple[object, dict[str, str], str]:
            choice_id = "steer"
            cases.append(CaseEvent(prompt, choice_id))
            return FakeMessage(prompt), custom_ids or {"Steer now": f"steer:{choice_id}"}, choice_id

        def make_interaction(
            *,
            bot: object,
            channel: object,
            message: object,
            user: object,
            custom_id: str,
        ) -> FakeInteraction:
            _ = (bot, channel)
            interactions.append(InteractionEvent(message, custom_id))
            return FakeInteraction(
                response=FakeResponse(),
                followup=FakeFollowup(),
                custom_id=custom_id,
                user=FakeUser() if not isinstance(user, FakeUser) else user,
            )

        async def handle_persistent_busy_choice_interaction(
            interaction: steer_case.SteerQaInteraction,
            custom_id: str,
            *,
            steering_runner: steer_case.SteeringRunner,
            steering_streamer: steer_case.SteeringStreamer,
        ) -> bool:
            _ = custom_id
            if not handler_result:
                return False
            target_thread_id = "qa-thread"
            steering_result = steering_runner("qa prompt", target_thread_id)
            await steering_streamer(
                object(),
                steering_result,
                target_thread_id,
                send_commentary_blocks=None,
                send_final_blocks=True,
            )
            if defer_response:
                interaction.response.deferred = True
                interaction.response.defer_kwargs.append({"ephemeral": ephemeral})
            interaction.followup.messages.append(followup_message)
            return True

        def delete_busy_choice_record(choice_id: str) -> None:
            deleted_choice_ids.append(choice_id)

        def get_mirrored_codex_thread_id(channel_id: int | None) -> str | None:
            return "qa-thread" if channel_id == 222 else None

        def make_steering_prompt_result(target_thread_id: str | None) -> steering.SteeringPromptResult:
            return steering.SteeringPromptResult(
                0,
                "[qa_delivery_verified]",
                target_thread_id=result_target_override or target_thread_id,
            )

        deps = steer_case.BusyChoiceSteerQaCaseDeps(
            send_case_button=send_case_button,
            make_interaction=make_interaction,
            handle_persistent_busy_choice_interaction=handle_persistent_busy_choice_interaction,
            delete_busy_choice_record=delete_busy_choice_record,
            get_mirrored_codex_thread_id=get_mirrored_codex_thread_id,
            make_steering_prompt_result=make_steering_prompt_result,
        )
        return deps, cases, interactions, deleted_choice_ids

    async def test_runs_busy_choice_steer_success_case(self) -> None:
        deps, cases, interactions, deleted_choice_ids = self.make_deps()

        line = await steer_case.run_busy_choice_steer_success_qa_case(
            bot=object(),
            channel=FakeChannel(),
            user=FakeUser(),
            deps=deps,
        )

        self.assertEqual(line, "steer_success: ok")
        self.assertEqual(cases, [CaseEvent("QA button steer success smoke", "steer")])
        self.assertEqual(interactions, [InteractionEvent(FakeMessage("QA button steer success smoke"), "steer:steer")])
        self.assertEqual(deleted_choice_ids, ["steer"])

    async def test_failed_predicates_return_failed_line(self) -> None:
        scenarios: list[
            tuple[
                str,
                Callable[[], tuple[steer_case.BusyChoiceSteerQaCaseDeps, list[CaseEvent], list[InteractionEvent], list[str]]],
            ]
        ] = [
            ("handler_result", lambda: self.make_deps(handler_result=False)),
            ("defer_response", lambda: self.make_deps(defer_response=False)),
            ("ephemeral", lambda: self.make_deps(ephemeral=False)),
            ("followup_message", lambda: self.make_deps(followup_message="wrong")),
            ("watched_target", lambda: self.make_deps(result_target_override="other-thread")),
        ]

        for name, make_deps in scenarios:
            with self.subTest(name=name):
                deps, _cases, _interactions, deleted_choice_ids = make_deps()

                line = await steer_case.run_busy_choice_steer_success_qa_case(
                    bot=object(),
                    channel=FakeChannel(),
                    user=FakeUser(),
                    deps=deps,
                )

                self.assertEqual(line, "steer_success: failed")
                self.assertEqual(deleted_choice_ids, ["steer"])

    async def test_missing_steer_now_custom_id_raises_key_error(self) -> None:
        deps, _cases, _interactions, _deleted_choice_ids = self.make_deps(custom_ids={"Ignore": "ignore:steer"})

        with self.assertRaises(KeyError):
            _ = await steer_case.run_busy_choice_steer_success_qa_case(
                bot=object(),
                channel=FakeChannel(),
                user=FakeUser(),
                deps=deps,
            )

    async def test_fake_response_records_production_defer_contract(self) -> None:
        response = FakeResponse()

        await response.defer(thinking=True, ephemeral=False)

        self.assertTrue(response.deferred)
        self.assertEqual(response.defer_kwargs, [{"thinking": True, "ephemeral": False}])


if __name__ == "__main__":
    _ = unittest.main()
