from __future__ import annotations

import unittest
from collections.abc import AsyncIterator

import codex_discord_stale_busy_components as stale_busy_components


class FakeButton:
    def __init__(self, custom_id: str | None) -> None:
        self.custom_id: str | None = custom_id


class FakeRow:
    def __init__(self, *buttons: FakeButton) -> None:
        self.children: list[FakeButton] = list(buttons)


class FakeAuthor:
    def __init__(self, *, bot: bool) -> None:
        self.bot: bool = bot


class FakeChannelRef:
    def __init__(self, channel_id: int) -> None:
        self.id: int = channel_id


class FakeMessage:
    def __init__(
        self,
        *,
        message_id: int,
        custom_ids: list[str | None],
        bot_author: bool = True,
        edit_fails: bool = False,
    ) -> None:
        self.id: int = message_id
        self.channel: FakeChannelRef = FakeChannelRef(222)
        self.author: FakeAuthor = FakeAuthor(bot=bot_author)
        self.components: list[FakeRow] = [
            FakeRow(*(FakeButton(custom_id) for custom_id in custom_ids))
        ]
        self.edited_views: list[None] = []
        self.edit_fails: bool = edit_fails

    async def edit(self, *, view: None = None) -> None:
        if self.edit_fails:
            raise RuntimeError("edit failed")
        self.edited_views.append(view)


class FakeHistoryChannel:
    def __init__(self, messages: list[FakeMessage]) -> None:
        self.messages: list[FakeMessage] = messages

    def history(self, *, limit: int) -> AsyncIterator[object]:
        async def iter_messages() -> AsyncIterator[object]:
            for message in self.messages[:limit]:
                yield message

        return iter_messages()


def record_getter(active_choice_ids: set[str]) -> stale_busy_components.BusyChoiceRecordGetter:
    def get_record(choice_id: str) -> stale_busy_components.BusyChoiceRecord | None:
        if choice_id in active_choice_ids:
            return {"choice_id": choice_id}
        return None

    return get_record


class StaleBusyChoiceComponentTests(unittest.IsolatedAsyncioTestCase):
    def test_has_active_busy_choice_custom_id_uses_record_lookup(self) -> None:
        choice_id = "0123456789abcdef01234567"

        self.assertTrue(
            stale_busy_components.has_active_busy_choice_custom_id(
                f"codex_busy:{choice_id}:steer",
                get_busy_choice_record=record_getter({choice_id}),
            )
        )
        self.assertFalse(
            stale_busy_components.has_active_busy_choice_custom_id(
                "codex_busy:not-valid:steer",
                get_busy_choice_record=record_getter({choice_id}),
            )
        )

    async def test_clear_removes_missing_busy_choice_record(self) -> None:
        logs: list[str] = []
        message = FakeMessage(
            message_id=123,
            custom_ids=["codex_busy:0123456789abcdef01234567:steer"],
        )

        cleared = await stale_busy_components.clear_stale_busy_choice_message_components(
            message,
            get_busy_choice_record=record_getter(set()),
            log_func=logs.append,
        )

        self.assertTrue(cleared)
        self.assertEqual(message.edited_views, [None])
        self.assertIn("stale_busy_choice_components_cleared message=123 channel=222", logs)

    async def test_clear_keeps_components_when_any_busy_choice_record_is_active(self) -> None:
        choice_id = "0123456789abcdef01234567"
        message = FakeMessage(
            message_id=124,
            custom_ids=[f"codex_busy:{choice_id}:queue"],
        )

        cleared = await stale_busy_components.clear_stale_busy_choice_message_components(
            message,
            get_busy_choice_record=record_getter({choice_id}),
            log_func=lambda text: None,
        )

        self.assertFalse(cleared)
        self.assertEqual(message.edited_views, [])

    async def test_clear_ignores_missing_or_malformed_busy_choice_ids(self) -> None:
        message = FakeMessage(message_id=125, custom_ids=[None, "codex_busy:bad:queue"])

        cleared = await stale_busy_components.clear_stale_busy_choice_message_components(
            message,
            get_busy_choice_record=record_getter(set()),
            log_func=lambda text: None,
        )

        self.assertFalse(cleared)
        self.assertEqual(message.edited_views, [])

    async def test_clear_logs_and_returns_false_when_edit_fails(self) -> None:
        logs: list[str] = []
        message = FakeMessage(
            message_id=126,
            custom_ids=["codex_busy:0123456789abcdef01234567:ignore"],
            edit_fails=True,
        )

        cleared = await stale_busy_components.clear_stale_busy_choice_message_components(
            message,
            get_busy_choice_record=record_getter(set()),
            log_func=logs.append,
        )

        self.assertFalse(cleared)
        self.assertIn("stale_busy_choice_components_clear_failed", "\n".join(logs))

    async def test_channel_cleanup_clears_only_bot_authored_stale_messages(self) -> None:
        active_choice_id = "aaaaaaaaaaaaaaaaaaaaaaaa"
        stale_bot = FakeMessage(
            message_id=127,
            custom_ids=["codex_busy:0123456789abcdef01234567:steer"],
        )
        active_bot = FakeMessage(
            message_id=128,
            custom_ids=[f"codex_busy:{active_choice_id}:queue"],
        )
        stale_human = FakeMessage(
            message_id=129,
            custom_ids=["codex_busy:bbbbbbbbbbbbbbbbbbbbbbbb:ignore"],
            bot_author=False,
        )

        cleared = await stale_busy_components.cleanup_stale_busy_choice_components_in_channel(
            FakeHistoryChannel([stale_bot, active_bot, stale_human]),
            get_busy_choice_record=record_getter({active_choice_id}),
            log_func=lambda text: None,
            limit=10,
        )

        self.assertEqual(cleared, 1)
        self.assertEqual(stale_bot.edited_views, [None])
        self.assertEqual(active_bot.edited_views, [])
        self.assertEqual(stale_human.edited_views, [])
