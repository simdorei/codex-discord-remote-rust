from __future__ import annotations

import unittest

import codex_discord_persistent_interactions as persistent_interactions
from codex_discord_steering import SteeringPromptResult


class FakeUser:
    def __init__(self, user_id: int = 242286902982606848) -> None:
        self.id: int = user_id


class FakeResponse:
    def __init__(self) -> None:
        self.deferred: bool = False
        self.defer_kwargs: list[dict[str, bool]] = []

    async def defer(self, thinking: bool = False, **kwargs: bool) -> None:
        self.deferred = True
        self.defer_kwargs.append({"thinking": thinking, **kwargs})


class FakeInteraction:
    def __init__(self, user_id: int = 242286902982606848) -> None:
        self.user: FakeUser = FakeUser(user_id)
        self.response: FakeResponse = FakeResponse()


class PersistentInteractionTests(unittest.IsolatedAsyncioTestCase):
    def make_approval_deps(
        self,
        *,
        allowed: bool = True,
        claim: bool = True,
        clears: list[str] | None = None,
        responses: list[tuple[str, bool, str]] | None = None,
        followups: list[tuple[str, str, int, str]] | None = None,
        streams: list[tuple[SteeringPromptResult | None, str]] | None = None,
        logs: list[str] | None = None,
        claims: list[str] | None = None,
    ) -> persistent_interactions.PersistentApprovalDeps:
        async def clear_components(
            interaction: persistent_interactions.PersistentInteraction,
            *,
            context: str,
        ) -> None:
            _ = interaction
            if clears is not None:
                clears.append(context)

        async def send_response(
            interaction: persistent_interactions.PersistentInteraction,
            content: str,
            *,
            ephemeral: bool,
            context: str,
        ) -> None:
            _ = interaction
            if responses is not None:
                responses.append((content, ephemeral, context))

        async def send_followup_chunks(
            interaction: persistent_interactions.PersistentInteraction,
            text: str,
            *,
            title: str,
            exit_code: int,
            log_prefix: str,
        ) -> None:
            _ = interaction
            if followups is not None:
                followups.append((text, title, exit_code, log_prefix))

        async def stream_post_approval_result(
            interaction: persistent_interactions.PersistentInteraction,
            watch_result: SteeringPromptResult | None,
            target_thread_id: str,
        ) -> bool:
            _ = interaction
            if streams is not None:
                streams.append((watch_result, target_thread_id))
            return True

        def claim_component(
            interaction: persistent_interactions.PersistentInteraction,
            custom_id: str,
        ) -> bool:
            _ = interaction
            if claims is not None:
                claims.append(custom_id)
            return claim

        return persistent_interactions.PersistentApprovalDeps(
            is_user_allowed=lambda user_id: allowed,
            claim_component=claim_component,
            clear_components=clear_components,
            send_response=send_response,
            send_followup_chunks=send_followup_chunks,
            make_watch_result=lambda target_thread_id: SteeringPromptResult(0, f"watch:{target_thread_id}", target_thread_id=target_thread_id),
            stream_post_approval_result=stream_post_approval_result,
            format_log_text_len=lambda value: str(len(str(value or ""))),
            log=(logs.append if logs is not None else lambda text: None),
        )

    def make_input_deps(
        self,
        *,
        allowed: bool = True,
        claim: bool = True,
        clears: list[str] | None = None,
        responses: list[tuple[str, bool, str]] | None = None,
        followups: list[tuple[str, str, int, str]] | None = None,
        logs: list[str] | None = None,
        claims: list[str] | None = None,
    ) -> persistent_interactions.PersistentInputChoiceDeps:
        async def clear_components(
            interaction: persistent_interactions.PersistentInteraction,
            *,
            context: str,
        ) -> None:
            _ = interaction
            if clears is not None:
                clears.append(context)

        async def send_response(
            interaction: persistent_interactions.PersistentInteraction,
            content: str,
            *,
            ephemeral: bool,
            context: str,
        ) -> None:
            _ = interaction
            if responses is not None:
                responses.append((content, ephemeral, context))

        async def send_followup_chunks(
            interaction: persistent_interactions.PersistentInteraction,
            text: str,
            *,
            title: str,
            exit_code: int,
            log_prefix: str,
        ) -> None:
            _ = interaction
            if followups is not None:
                followups.append((text, title, exit_code, log_prefix))

        def claim_component(
            interaction: persistent_interactions.PersistentInteraction,
            custom_id: str,
        ) -> bool:
            _ = interaction
            if claims is not None:
                claims.append(custom_id)
            return claim

        return persistent_interactions.PersistentInputChoiceDeps(
            is_user_allowed=lambda user_id: allowed,
            claim_component=claim_component,
            clear_components=clear_components,
            send_response=send_response,
            send_followup_chunks=send_followup_chunks,
            format_log_text_len=lambda value: str(len(str(value or ""))),
            log=(logs.append if logs is not None else lambda text: None),
        )

    async def test_approval_success_submits_follows_up_and_streams(self) -> None:
        clears: list[str] = []
        followups: list[tuple[str, str, int, str]] = []
        streams: list[tuple[SteeringPromptResult | None, str]] = []
        watch_result = SteeringPromptResult(0, "watch:thread-1", target_thread_id="thread-1")
        submitted: list[tuple[str, str]] = []
        logs: list[str] = []

        def submitter(target_thread_id: str, answer: str) -> tuple[int, str]:
            submitted.append((target_thread_id, answer))
            return 0, "approved"

        interaction = FakeInteraction()
        handled = await persistent_interactions.handle_persistent_approval_interaction(
            interaction,
            "codex_approval:thread-1:2",
            approval_submitter=submitter,
            deps=self.make_approval_deps(
                clears=clears,
                followups=followups,
                streams=streams,
                logs=logs,
            ),
        )

        self.assertTrue(handled)
        self.assertTrue(interaction.response.deferred)
        self.assertEqual(clears, ["approval_persistent"])
        self.assertEqual(submitted, [("thread-1", "2")])
        self.assertEqual(followups, [("Approval submitted\n\napproved", "Approval", 0, "button_response")])
        self.assertEqual(streams, [(watch_result, "thread-1")])
        self.assertIn("approval_persistent_done exit=0 target=thread-1 answer_len=1", logs)

    async def test_input_success_submits_and_follows_up(self) -> None:
        clears: list[str] = []
        followups: list[tuple[str, str, int, str]] = []
        submitted: list[tuple[str, str]] = []

        def submitter(target_thread_id: str, value: str) -> tuple[int, str]:
            submitted.append((target_thread_id, value))
            return 0, "answered"

        interaction = FakeInteraction()
        handled = await persistent_interactions.handle_persistent_input_choice_interaction(
            interaction,
            "codex_input:thread-1:choice-1",
            input_submitter=submitter,
            deps=self.make_input_deps(clears=clears, followups=followups),
        )

        self.assertTrue(handled)
        self.assertTrue(interaction.response.deferred)
        self.assertEqual(clears, ["input_choice_persistent"])
        self.assertEqual(submitted, [("thread-1", "choice-1")])
        self.assertEqual(followups, [("Input submitted\n\nanswered", "Input", 0, "button_response")])

    async def test_invalid_custom_ids_return_unhandled(self) -> None:
        approval = await persistent_interactions.handle_persistent_approval_interaction(
            FakeInteraction(),
            "not-approval",
            approval_submitter=lambda target_thread_id, answer: (0, "unused"),
            deps=self.make_approval_deps(),
        )
        input_choice = await persistent_interactions.handle_persistent_input_choice_interaction(
            FakeInteraction(),
            "not-input",
            input_submitter=lambda target_thread_id, value: (0, "unused"),
            deps=self.make_input_deps(),
        )

        self.assertFalse(approval)
        self.assertFalse(input_choice)

    async def test_denied_input_sends_response_without_claiming_or_deferring(self) -> None:
        responses: list[tuple[str, bool, str]] = []
        claims: list[str] = []
        interaction = FakeInteraction(user_id=999)

        handled = await persistent_interactions.handle_persistent_input_choice_interaction(
            interaction,
            "codex_input:thread-1:choice-1",
            input_submitter=lambda target_thread_id, value: (0, "unused"),
            deps=self.make_input_deps(allowed=False, responses=responses, claims=claims),
        )

        self.assertTrue(handled)
        self.assertFalse(interaction.response.deferred)
        self.assertEqual(claims, [])
        self.assertEqual(
            responses,
            [("This user is not allowed.", True, "input_choice_persistent_denied")],
        )

    async def test_already_claimed_approval_clears_and_does_not_submit(self) -> None:
        clears: list[str] = []
        responses: list[tuple[str, bool, str]] = []
        submitted: list[tuple[str, str]] = []

        def submitter(target_thread_id: str, answer: str) -> tuple[int, str]:
            submitted.append((target_thread_id, answer))
            return 0, "unused"

        handled = await persistent_interactions.handle_persistent_approval_interaction(
            FakeInteraction(),
            "codex_approval:thread-1:2",
            approval_submitter=submitter,
            deps=self.make_approval_deps(claim=False, clears=clears, responses=responses),
        )

        self.assertTrue(handled)
        self.assertEqual(submitted, [])
        self.assertEqual(clears, ["approval_persistent_already_handled"])
        self.assertEqual(
            responses,
            [
                (
                    "This approval choice was already handled.",
                    True,
                    "approval_persistent_already_handled",
                )
            ],
        )
