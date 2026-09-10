from __future__ import annotations

import unittest

import codex_discord_text as text


class DiscordTextTests(unittest.TestCase):
    def test_format_percent_formats_numeric_values(self) -> None:
        self.assertEqual(text.format_percent(12), "12.0%")
        self.assertEqual(text.format_percent(12.34), "12.3%")

    def test_format_percent_parses_numeric_strings(self) -> None:
        self.assertEqual(text.format_percent("45.67"), "45.7%")

    def test_format_percent_returns_dash_for_invalid_values(self) -> None:
        self.assertEqual(text.format_percent("not-a-number"), "-")
        self.assertEqual(text.format_percent(None), "-")


if __name__ == "__main__":
    _ = unittest.main()
