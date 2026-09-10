from __future__ import annotations

import unittest
from typing import ClassVar

import codex_discord_socket_event_log as socket_event_log


class _SeenOwner:
    def has_seen(self, key: str) -> bool:
        raw_seen = getattr(self, "_logged_socket_event_ids", None)
        return isinstance(raw_seen, dict) and key in raw_seen


class _SlotOnlyOwner:
    __slots__: ClassVar[tuple[str, ...]] = ()


class SocketEventLogTests(unittest.TestCase):
    def test_get_socket_event_log_key_uses_sequence_when_present(self) -> None:
        self.assertEqual(
            socket_event_log.get_socket_event_log_key({"t": "MESSAGE_CREATE", "s": 10}),
            "MESSAGE_CREATE:s:10",
        )

    def test_get_socket_event_log_key_uses_data_id_when_sequence_missing(self) -> None:
        self.assertEqual(
            socket_event_log.get_socket_event_log_key(
                {"t": "INTERACTION_CREATE", "d": {"id": "abc"}}
            ),
            "INTERACTION_CREATE:id:abc",
        )

    def test_get_socket_event_log_key_returns_none_without_event_type(self) -> None:
        self.assertIsNone(socket_event_log.get_socket_event_log_key({"s": 10}))

    def test_get_socket_event_log_key_returns_none_without_stable_id(self) -> None:
        self.assertIsNone(socket_event_log.get_socket_event_log_key({"t": "READY"}))

    def test_claim_socket_event_log_records_first_stable_event(self) -> None:
        owner = _SeenOwner()
        payload: socket_event_log.SocketEventData = {"t": "MESSAGE_CREATE", "s": 10}

        claimed = socket_event_log.claim_socket_event_log(owner, payload, limit=10)

        self.assertTrue(claimed)
        self.assertTrue(owner.has_seen("MESSAGE_CREATE:s:10"))

    def test_claim_socket_event_log_rejects_duplicate_event(self) -> None:
        owner = _SeenOwner()
        payload: socket_event_log.SocketEventData = {"t": "MESSAGE_CREATE", "s": 10}

        first_claimed = socket_event_log.claim_socket_event_log(owner, payload, limit=10)
        second_claimed = socket_event_log.claim_socket_event_log(owner, payload, limit=10)

        self.assertTrue(first_claimed)
        self.assertFalse(second_claimed)

    def test_claim_socket_event_log_allows_unkeyed_or_unwritable_owner(self) -> None:
        unkeyed_payload: socket_event_log.SocketEventData = {"t": "READY"}

        self.assertTrue(
            socket_event_log.claim_socket_event_log(_SeenOwner(), unkeyed_payload, limit=10)
        )
        self.assertTrue(
            socket_event_log.claim_socket_event_log(
                _SlotOnlyOwner(),
                {"t": "MESSAGE_CREATE", "s": 10},
                limit=10,
            )
        )

    def test_format_socket_interaction_user_prefers_top_level_user(self) -> None:
        self.assertEqual(
            socket_event_log.format_socket_interaction_user(
                {
                    "user": {"id": "user-1"},
                    "member": {"user": {"id": "member-1"}},
                }
            ),
            "user-1",
        )

    def test_format_socket_interaction_user_falls_back_to_member_user(self) -> None:
        self.assertEqual(
            socket_event_log.format_socket_interaction_user(
                {"member": {"user": {"id": "member-1"}}}
            ),
            "member-1",
        )

    def test_format_socket_interaction_user_returns_dash_without_user_id(self) -> None:
        self.assertEqual(socket_event_log.format_socket_interaction_user({}), "-")

    def test_format_socket_payload_log_lines_formats_tracked_message(self) -> None:
        payload: socket_event_log.SocketEventData = {
            "t": "MESSAGE_CREATE",
            "d": {
                "channel_id": "333",
                "guild_id": "444",
                "author": {"id": "555", "bot": True},
                "content": "hello",
            },
        }

        lines = socket_event_log.format_socket_payload_log_lines(
            payload,
            claim_event=lambda _payload: True,
            track_message_channel=lambda channel_id: (channel_id == 333, "unit"),
        )

        self.assertEqual(
            lines,
            (
                "socket_message_create channel=333 tracked=True "
                + "source=unit guild=444 author=555 bot=True content_len=5",
            ),
        )

    def test_format_socket_payload_log_lines_formats_untracked_message(self) -> None:
        payload: socket_event_log.SocketEventData = {
            "t": "MESSAGE_CREATE",
            "d": {"channel_id": "333", "guild_id": "444", "content": "hello"},
        }

        lines = socket_event_log.format_socket_payload_log_lines(
            payload,
            claim_event=lambda _payload: True,
            track_message_channel=lambda _channel_id: (False, "not-tracked"),
        )

        self.assertEqual(
            lines,
            ("socket_message_create_untracked channel=333 guild=444 source=not-tracked",),
        )

    def test_format_socket_payload_log_lines_formats_interaction_create(self) -> None:
        payload: socket_event_log.SocketEventData = {
            "t": "INTERACTION_CREATE",
            "d": {
                "channel_id": "333",
                "guild_id": "444",
                "type": 2,
                "member": {"user": {"id": "555"}},
                "data": {"name": "ask"},
            },
        }

        lines = socket_event_log.format_socket_payload_log_lines(
            payload,
            claim_event=lambda _payload: True,
            track_message_channel=lambda _channel_id: (False, "-"),
        )

        self.assertEqual(
            lines,
            ("socket_interaction_create channel=333 guild=444 user=555 type=2 command=ask",),
        )

    def test_format_socket_payload_log_lines_skips_claimed_payload(self) -> None:
        payload: socket_event_log.SocketEventData = {
            "t": "MESSAGE_CREATE",
            "d": {"channel_id": "333"},
        }

        lines = socket_event_log.format_socket_payload_log_lines(
            payload,
            claim_event=lambda _payload: False,
            track_message_channel=lambda _channel_id: (True, "unit"),
        )

        self.assertEqual(lines, ())


if __name__ == "__main__":
    _ = unittest.main()
