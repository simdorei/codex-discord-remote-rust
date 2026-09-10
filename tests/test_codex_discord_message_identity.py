from __future__ import annotations

import unittest

import codex_discord_message_identity as message_identity


class IntLikeMessageId:
    def __int__(self) -> int:
        return 1234


class MessageIdentityTests(unittest.TestCase):
    def test_coerce_discord_message_id_returns_none_for_missing_id(self) -> None:
        self.assertIsNone(message_identity.coerce_discord_message_id(None))

    def test_coerce_discord_message_id_accepts_int_and_numeric_text(self) -> None:
        self.assertEqual(message_identity.coerce_discord_message_id(123), 123)
        self.assertEqual(message_identity.coerce_discord_message_id("456"), 456)

    def test_coerce_discord_message_id_accepts_int_like_value(self) -> None:
        self.assertEqual(message_identity.coerce_discord_message_id(IntLikeMessageId()), 1234)

    def test_coerce_discord_message_id_returns_none_for_invalid_value(self) -> None:
        self.assertIsNone(message_identity.coerce_discord_message_id("invalid"))


if __name__ == "__main__":
    _ = unittest.main()
