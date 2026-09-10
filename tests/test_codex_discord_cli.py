from __future__ import annotations

import unittest

import codex_discord_cli as discord_cli


class DiscordCliTests(unittest.TestCase):
    def test_build_parser_defaults_message_content_to_enabled(self) -> None:
        parsed = discord_cli.build_parser().parse_args([])

        self.assertEqual(repr(vars(parsed)), "{'no_message_content': False}")

    def test_build_parser_accepts_no_message_content_flag(self) -> None:
        parsed = discord_cli.build_parser().parse_args(["--no-message-content"])

        self.assertEqual(repr(vars(parsed)), "{'no_message_content': True}")


if __name__ == "__main__":
    _ = unittest.main()
