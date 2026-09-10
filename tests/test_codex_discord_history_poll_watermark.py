from __future__ import annotations

import unittest
from collections.abc import AsyncIterator
from dataclasses import dataclass, field
from datetime import datetime, timedelta, timezone

import codex_discord_history_poll as history_poll
from codex_discord_history_poll import PollHistoryChannelDeps, poll_history_channel


@dataclass(frozen=True, slots=True)
class Message:
    id: int
    created_at: datetime


@dataclass(slots=True)
class HistoryChannel:
    messages: list[Message]
    events: list[str]

    def history(self, *, limit: int) -> AsyncIterator[Message]:
        async def iterator() -> AsyncIterator[Message]:
            self.events.append("history")
            for message in self.messages[:limit]:
                yield message

        return iterator()


@dataclass(slots=True)
class PollState:
    channel: HistoryChannel
    now: datetime
    primed: bool = False
    watermark: tuple[datetime, int] | None = None
    processed: list[int] = field(default_factory=list)
    logs: list[str] = field(default_factory=list)


class IntLikeMessageId:
    def __int__(self) -> int:
        return 102


def _deps(state: PollState) -> PollHistoryChannelDeps[Message]:
    async def fetch_channel(_channel_id: int) -> HistoryChannel:
        raise AssertionError("fetch not expected")

    async def process_message(message: Message, _channel_id: int) -> None:
        state.processed.append(message.id)

    def mark_primed(_channel_id: int) -> None:
        state.primed = True

    def set_watermark(_channel_id: int, value: tuple[datetime, int]) -> None:
        state.watermark = value

    def now() -> datetime:
        state.channel.events.append("now")
        return state.now

    return PollHistoryChannelDeps(
        get_cached_channel_or_thread=lambda _channel_id: (state.channel, "test_cache"),
        fetch_channel=fetch_channel,
        delivery_exceptions=(RuntimeError,),
        history_limit=10,
        is_primed_channel=lambda _channel_id: state.primed,
        mark_primed_channel=mark_primed,
        get_channel_watermark=lambda _channel_id: state.watermark,
        set_channel_watermark=set_watermark,
        now=now,
        claim_message=lambda _message: True,
        process_history_poll_message=process_message,
        log=state.logs.append,
    )


class HistoryPollWatermarkTests(unittest.IsolatedAsyncioTestCase):
    async def test_message_watermark_preserves_supported_id_conversions(self) -> None:
        created_at = datetime(2026, 7, 22, 5, 0, tzinfo=timezone.utc)

        string_id_message = type("Message", (), {"id": "101", "created_at": created_at})()
        int_like_message = type(
            "Message",
            (),
            {"id": IntLikeMessageId(), "created_at": created_at},
        )()

        self.assertEqual(history_poll.message_watermark(string_id_message), (created_at, 101))
        self.assertEqual(history_poll.message_watermark(int_like_message), (created_at, 102))

    async def test_message_watermark_rejects_missing_or_invalid_id(self) -> None:
        created_at = datetime(2026, 7, 22, 5, 0, tzinfo=timezone.utc)
        missing_id_message = type("Message", (), {"created_at": created_at})()
        invalid_id_message = type("Message", (), {"id": "invalid", "created_at": created_at})()

        self.assertIsNone(history_poll.message_watermark(missing_id_message))
        self.assertIsNone(history_poll.message_watermark(invalid_id_message))

    async def test_first_prime_captures_boundary_before_fetch_and_only_processes_newer_message(self) -> None:
        boundary = datetime(2026, 7, 22, 5, 0, tzinfo=timezone.utc)
        events: list[str] = []
        old_message = Message(100, boundary - timedelta(seconds=1))
        new_message = Message(101, boundary + timedelta(seconds=1))
        channel = HistoryChannel([new_message, old_message], events)
        state = PollState(channel, boundary)

        await poll_history_channel("allowed", 333, deps=_deps(state))

        self.assertEqual(events, ["now", "history"])
        self.assertEqual(state.processed, [101])
        self.assertEqual(state.watermark, (new_message.created_at, new_message.id))
        self.assertIn("fetched_messages=2", state.logs[0])
        self.assertIn("claimed_messages=2", state.logs[0])
        self.assertIn("eligible_messages=1", state.logs[0])
        self.assertIn("discarded_messages=1", state.logs[0])

    async def test_primed_channel_without_watermark_reprime_drops_snapshot(self) -> None:
        boundary = datetime(2026, 7, 22, 5, 0, tzinfo=timezone.utc)
        message = Message(101, boundary + timedelta(seconds=1))
        state = PollState(
            HistoryChannel([message], []),
            boundary,
            primed=True,
            watermark=None,
        )

        await poll_history_channel("allowed", 333, deps=_deps(state))

        self.assertEqual(state.processed, [])
        self.assertEqual(state.watermark, (message.created_at, message.id))
        self.assertIn("history_poll_reprimed", state.logs[0])
        self.assertIn("reason=missing_watermark", state.logs[0])
        self.assertIn("discarded_messages=1", state.logs[0])

    async def test_same_timestamp_uses_message_id_as_tiebreaker(self) -> None:
        created_at = datetime(2026, 7, 22, 5, 0, tzinfo=timezone.utc)
        message = Message(101, created_at)
        state = PollState(
            HistoryChannel([message], []),
            created_at,
            primed=True,
            watermark=(created_at, 100),
        )

        await poll_history_channel("allowed", 333, deps=_deps(state))

        self.assertEqual(state.processed, [101])
        self.assertEqual(state.watermark, (created_at, 101))


if __name__ == "__main__":
    _ = unittest.main()
