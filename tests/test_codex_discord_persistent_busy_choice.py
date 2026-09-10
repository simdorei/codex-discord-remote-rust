from __future__ import annotations

import unittest
from datetime import datetime, timezone

import codex_discord_persistent_busy_choice as persistent_busy_choice


def make_record(
    *,
    owner_user_id: int = 242286902982606848,
    target_thread_id: str | None = "thread-1",
    allow_steer: bool = True,
) -> persistent_busy_choice.BusyChoiceRecord:
    return {
        "owner_user_id": owner_user_id,
        "target_thread_id": target_thread_id,
        "allow_steer": allow_steer,
        "prompt": "please queue",
        "channel_id": 222,
        "created_at": 123.0,
    }


class PersistentBusyChoiceResolverTests(unittest.TestCase):
    def test_make_persistent_busy_source_message_builds_owner_and_channel(self) -> None:
        channel = object()
        source = persistent_busy_choice.make_persistent_busy_source_message(
            {"owner_user_id": "123", "created_at": 123.0},
            channel,
        )

        self.assertEqual(source.author.id, 123)
        self.assertIs(source.channel, channel)
        self.assertEqual(source.created_at, datetime.fromtimestamp(123.0, tz=timezone.utc))

    def test_invalid_custom_id_resolves_unhandled(self) -> None:
        resolution = persistent_busy_choice.resolve_persistent_busy_choice(
            "not-busy",
            user_id=1,
            get_busy_choice_record=lambda choice_id: make_record(),
        )

        self.assertEqual(resolution.status, "unhandled")
        self.assertEqual(resolution.choice_id, "")

    def test_missing_record_resolves_missing(self) -> None:
        resolution = persistent_busy_choice.resolve_persistent_busy_choice(
            "codex_busy:0123456789abcdef01234567:steer",
            user_id=1,
            get_busy_choice_record=lambda choice_id: None,
        )

        self.assertEqual(resolution.status, "missing")
        self.assertEqual(resolution.choice_id, "0123456789abcdef01234567")
        self.assertEqual(resolution.action, "steer")
        self.assertIsNone(resolution.record)

    def test_wrong_user_resolves_denied_with_owner_and_target(self) -> None:
        resolution = persistent_busy_choice.resolve_persistent_busy_choice(
            "codex_busy:0123456789abcdef01234567:queue",
            user_id=999,
            get_busy_choice_record=lambda choice_id: make_record(owner_user_id=111),
        )

        self.assertEqual(resolution.status, "denied")
        self.assertEqual(resolution.owner_user_id, 111)
        self.assertEqual(resolution.target_thread_id, "thread-1")

    def test_steer_not_allowed_keeps_record_available_for_queue(self) -> None:
        record = make_record(allow_steer=False)

        resolution = persistent_busy_choice.resolve_persistent_busy_choice(
            "codex_busy:0123456789abcdef01234567:steer",
            user_id=242286902982606848,
            get_busy_choice_record=lambda choice_id: record,
        )

        self.assertEqual(resolution.status, "steer_not_allowed")
        self.assertIs(resolution.record, record)

    def test_queue_with_owner_resolves_ready(self) -> None:
        record = make_record(allow_steer=False)

        resolution = persistent_busy_choice.resolve_persistent_busy_choice(
            "codex_busy:0123456789abcdef01234567:queue",
            user_id=242286902982606848,
            get_busy_choice_record=lambda choice_id: record,
        )

        self.assertEqual(resolution.status, "ready")
        self.assertEqual(resolution.target_thread_id, "thread-1")
        self.assertIs(resolution.record, record)


class PersistentBusyChoiceIgnoreTests(unittest.IsolatedAsyncioTestCase):
    async def test_ignore_logs_clears_and_sends_response(self) -> None:
        clears: list[str] = []
        responses: list[tuple[str, str]] = []
        logs: list[str] = []
        target = object()

        async def clear_components(
            interaction: persistent_busy_choice.PersistentBusyInteraction,
            *,
            context: str,
        ) -> None:
            self.assertIs(interaction, target)
            clears.append(context)

        async def send_response(
            interaction: persistent_busy_choice.PersistentBusyInteraction,
            content: str,
            *,
            context: str,
        ) -> None:
            self.assertIs(interaction, target)
            responses.append((content, context))

        handled = await persistent_busy_choice.handle_persistent_busy_ignore(
            target,
            user_id=242286902982606848,
            choice_id="0123456789abcdef01234567",
            target_thread_id="thread-1",
            deps=persistent_busy_choice.PersistentBusyIgnoreDeps(
                clear_components=clear_components,
                send_response=send_response,
                log=logs.append,
            ),
        )

        self.assertTrue(handled)
        self.assertEqual(clears, ["busy_choice_ignore"])
        self.assertEqual(responses, [("Ignored.", "busy_choice_persistent_ignore")])
        expected_log = (
            "busy_choice_persistent_ignore "
            "user=242286902982606848 choice=0123456789abcdef01234567 target=thread-1"
        )
        self.assertEqual(
            logs,
            [expected_log],
        )

    async def test_ignore_logs_dash_for_missing_target_thread(self) -> None:
        logs: list[str] = []

        async def clear_components(
            interaction: persistent_busy_choice.PersistentBusyInteraction,
            *,
            context: str,
        ) -> None:
            _ = (interaction, context)

        async def send_response(
            interaction: persistent_busy_choice.PersistentBusyInteraction,
            content: str,
            *,
            context: str,
        ) -> None:
            _ = (interaction, content, context)

        _ = await persistent_busy_choice.handle_persistent_busy_ignore(
            object(),
            user_id=1,
            choice_id="0123456789abcdef01234567",
            target_thread_id=None,
            deps=persistent_busy_choice.PersistentBusyIgnoreDeps(
                clear_components=clear_components,
                send_response=send_response,
                log=logs.append,
            ),
        )

        self.assertEqual(
            logs,
            ["busy_choice_persistent_ignore user=1 choice=0123456789abcdef01234567 target=-"],
        )


class PersistentBusyChoiceChannelUnavailableTests(unittest.IsolatedAsyncioTestCase):
    async def test_channel_unavailable_sends_followup_and_logs_target(self) -> None:
        followups: list[tuple[str, str, str]] = []
        logs: list[str] = []
        target = object()

        async def send_followup(
            interaction: persistent_busy_choice.PersistentBusyInteraction,
            content: str,
            *,
            log_prefix: str,
            context: str,
        ) -> None:
            self.assertIs(interaction, target)
            followups.append((content, log_prefix, context))

        handled = await persistent_busy_choice.handle_persistent_busy_channel_unavailable(
            target,
            action="queue",
            choice_id="0123456789abcdef01234567",
            target_thread_id="thread-1",
            deps=persistent_busy_choice.PersistentBusyChannelUnavailableDeps(
                send_followup=send_followup,
                log=logs.append,
            ),
        )

        expected_content = "Discord channel is unavailable. Send the message again to get fresh controls."
        expected_log = (
            "busy_choice_persistent_channel_unavailable "
            "action=queue choice=0123456789abcdef01234567 target=thread-1"
        )
        self.assertTrue(handled)
        self.assertEqual(followups, [(expected_content, "button_followup", "persistent_channel_unavailable")])
        self.assertEqual(logs, [expected_log])

    async def test_channel_unavailable_logs_dash_for_missing_target(self) -> None:
        logs: list[str] = []

        async def send_followup(
            interaction: persistent_busy_choice.PersistentBusyInteraction,
            content: str,
            *,
            log_prefix: str,
            context: str,
        ) -> None:
            _ = (interaction, content, log_prefix, context)

        _ = await persistent_busy_choice.handle_persistent_busy_channel_unavailable(
            object(),
            action="queue",
            choice_id="0123456789abcdef01234567",
            target_thread_id=None,
            deps=persistent_busy_choice.PersistentBusyChannelUnavailableDeps(
                send_followup=send_followup,
                log=logs.append,
            ),
        )

        self.assertEqual(
            logs,
            ["busy_choice_persistent_channel_unavailable action=queue choice=0123456789abcdef01234567 target=-"],
        )

    async def test_channel_unavailable_preserves_steer_action_label(self) -> None:
        logs: list[str] = []

        async def send_followup(
            interaction: persistent_busy_choice.PersistentBusyInteraction,
            content: str,
            *,
            log_prefix: str,
            context: str,
        ) -> None:
            _ = (interaction, content, log_prefix, context)

        _ = await persistent_busy_choice.handle_persistent_busy_channel_unavailable(
            object(),
            action="steer",
            choice_id="0123456789abcdef01234567",
            target_thread_id="thread-1",
            deps=persistent_busy_choice.PersistentBusyChannelUnavailableDeps(
                send_followup=send_followup,
                log=logs.append,
            ),
        )

        self.assertEqual(
            logs,
            ["busy_choice_persistent_channel_unavailable action=steer choice=0123456789abcdef01234567 target=thread-1"],
        )
