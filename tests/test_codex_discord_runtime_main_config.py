from __future__ import annotations

import os
import unittest
from unittest.mock import patch

import codex_discord_runtime_config as runtime_config


class RuntimeMainConfigTests(unittest.TestCase):
    def test_discord_allow_all_channels_enabled_defaults_to_false(self) -> None:
        with patch.dict(os.environ, {}, clear=True):
            self.assertFalse(runtime_config.discord_allow_all_channels_enabled())

    def test_discord_allow_all_channels_enabled_reads_env(self) -> None:
        with patch.dict(os.environ, {"DISCORD_ALLOW_ALL_CHANNELS": "yes"}, clear=True):
            self.assertTrue(runtime_config.discord_allow_all_channels_enabled())
        with patch.dict(os.environ, {"DISCORD_ALLOW_ALL_CHANNELS": "0"}, clear=True):
            self.assertFalse(runtime_config.discord_allow_all_channels_enabled())

    def test_discord_message_content_enabled_defaults_to_true(self) -> None:
        with patch.dict(os.environ, {}, clear=True):
            self.assertTrue(runtime_config.discord_message_content_enabled())

    def test_discord_message_content_enabled_reads_env(self) -> None:
        with patch.dict(os.environ, {"DISCORD_ENABLE_MESSAGE_CONTENT": "0"}, clear=True):
            self.assertFalse(runtime_config.discord_message_content_enabled())
        with patch.dict(os.environ, {"DISCORD_ENABLE_MESSAGE_CONTENT": "yes"}, clear=True):
            self.assertTrue(runtime_config.discord_message_content_enabled())

    def test_get_discord_allowed_channel_ids_parses_int_set(self) -> None:
        with patch.dict(os.environ, {"DISCORD_ALLOWED_CHANNEL_IDS": "1, x, 2"}, clear=True):
            self.assertEqual(runtime_config.get_discord_allowed_channel_ids(), {1, 2})

    def test_get_discord_allowed_user_ids_parses_int_set(self) -> None:
        with patch.dict(os.environ, {"DISCORD_ALLOWED_USER_IDS": "3, 4, bad"}, clear=True):
            self.assertEqual(runtime_config.get_discord_allowed_user_ids(), {3, 4})

    def test_get_plain_ask_mention_user_ids_parses_int_set(self) -> None:
        with patch.dict(os.environ, {"DISCORD_PLAIN_ASK_MENTION_USER_IDS": "5,6,nope"}, clear=True):
            self.assertEqual(runtime_config.get_plain_ask_mention_user_ids(), {5, 6})

    def test_get_discord_guild_id_returns_none_for_blank_env(self) -> None:
        with patch.dict(os.environ, {}, clear=True):
            self.assertIsNone(runtime_config.get_discord_guild_id())

    def test_get_discord_guild_id_parses_env(self) -> None:
        with patch.dict(os.environ, {"DISCORD_GUILD_ID": "123"}, clear=True):
            self.assertEqual(runtime_config.get_discord_guild_id(), 123)

    def test_get_startup_channel_id_prefers_env(self) -> None:
        with patch.dict(os.environ, {"DISCORD_STARTUP_CHANNEL_ID": "456"}, clear=True):
            self.assertEqual(runtime_config.get_startup_channel_id({111}), 456)

    def test_get_startup_channel_id_uses_single_allowed_channel_fallback(self) -> None:
        with patch.dict(os.environ, {}, clear=True):
            self.assertEqual(runtime_config.get_startup_channel_id({789}), 789)

    def test_get_startup_channel_id_returns_none_for_multiple_allowed_channels(self) -> None:
        with patch.dict(os.environ, {}, clear=True):
            self.assertIsNone(runtime_config.get_startup_channel_id({1, 2}))


if __name__ == "__main__":
    _ = unittest.main()
