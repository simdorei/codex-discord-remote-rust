from __future__ import annotations

import unittest

import codex_discord_id_values as id_values


class DiscordIdValuesTests(unittest.TestCase):
    def test_coerce_discord_id_value_accepts_int_string_and_ascii_bytes(self) -> None:
        self.assertEqual(id_values.coerce_discord_id_value(123), 123)
        self.assertEqual(id_values.coerce_discord_id_value("456"), 456)
        self.assertEqual(id_values.coerce_discord_id_value(b"789"), 789)
        self.assertEqual(id_values.coerce_discord_id_value(bytearray(b"321")), 321)

    def test_coerce_discord_id_value_rejects_none_invalid_and_non_ascii(self) -> None:
        self.assertIsNone(id_values.coerce_discord_id_value(None))
        self.assertIsNone(id_values.coerce_discord_id_value("not-a-number"))
        self.assertIsNone(id_values.coerce_discord_id_value(bytes([0xFF])))


if __name__ == "__main__":
    _ = unittest.main()
