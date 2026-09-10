from __future__ import annotations

import unittest
from types import SimpleNamespace
from typing import cast

import codex_discord_message_mentions as mentions


def _message(value: SimpleNamespace) -> mentions.MessageWithMentions:
    return cast(mentions.MessageWithMentions, cast(object, value))


def _bot_message(value: SimpleNamespace) -> mentions.BotMessageWithMentions:
    return cast(mentions.BotMessageWithMentions, cast(object, value))


def _client(value: SimpleNamespace) -> mentions.DiscordClientWithMentions:
    return cast(mentions.DiscordClientWithMentions, cast(object, value))


class DiscordMessageMentionTests(unittest.TestCase):
    def test_strip_required_mentions_preserves_roles_and_other_users(self) -> None:
        prompt, matched = mentions.strip_required_plain_ask_mentions(
            "<@!1500506752234422322> ask <@&1500506752234422322> <@999>",
            {1500506752234422322},
        )

        self.assertTrue(matched)
        self.assertEqual(prompt, "ask <@&1500506752234422322> <@999>")

    def test_message_mention_ids_coerce_raw_and_user_mentions(self) -> None:
        message = SimpleNamespace(
            raw_mentions=["1", b"2", bytearray(b"3"), "bad"],
            mentions=[
                SimpleNamespace(id="4", bot=False),
                SimpleNamespace(id=b"5", bot=True),
                SimpleNamespace(id=None, bot=False),
            ],
        )

        self.assertEqual(
            mentions.get_discord_message_mention_ids(_message(message)),
            {1, 2, 3, 4, 5},
        )

    def test_bridge_user_ids_include_configured_and_client_user(self) -> None:
        client = SimpleNamespace(
            plain_ask_mention_user_ids={1, "2", b"3", "bad"},
            user=SimpleNamespace(id="4", bot=True),
        )

        self.assertEqual(mentions.get_bridge_mention_user_ids(_client(client)), {1, 2, 3, 4})

    def test_bot_bridge_and_other_bot_detection(self) -> None:
        message = SimpleNamespace(
            author=SimpleNamespace(bot=True),
            raw_mentions=[1511380398914142379, 1500506752234422322],
            mentions=[
                SimpleNamespace(id=1511380398914142379, bot=True),
                SimpleNamespace(id=1500506752234422322, bot=True),
            ],
        )
        client = SimpleNamespace(
            plain_ask_mention_user_ids={1511380398914142379},
            user=SimpleNamespace(id=999, bot=True),
        )

        self.assertTrue(mentions.message_mentions_bridge_user(_message(message), _client(client)))
        self.assertTrue(mentions.is_bot_authored_bridge_mention(_bot_message(message), _client(client)))
        self.assertTrue(
            mentions.message_mentions_other_bot(
                _message(message),
                {1511380398914142379},
            )
        )
        self.assertFalse(
            mentions.message_mentions_other_bot(
                _message(message),
                {1511380398914142379, 1500506752234422322},
            )
        )

    def test_strip_required_plain_ask_mentions_strip_only_user_mentions(self) -> None:
        prompt, matched = mentions.strip_required_plain_ask_mentions(
            "<@!1500506752234422322> ask <@&1500506752234422322> <@999>",
            {1500506752234422322},
        )

        self.assertTrue(matched)
        self.assertEqual(prompt, "ask <@&1500506752234422322> <@999>")

    def test_strip_required_plain_ask_mentions_preserves_other_mentions(self) -> None:
        prompt, matched = mentions.strip_required_plain_ask_mentions(
            "<@1500506752234422322> ask <@999> now",
            {1500506752234422322},
        )

        self.assertTrue(matched)
        self.assertEqual(prompt, "ask <@999> now")

    def test_strip_required_plain_ask_mentions_accepts_nickname_mention(self) -> None:
        prompt, matched = mentions.strip_required_plain_ask_mentions(
            "<@!1500506752234422322> ask now",
            {1500506752234422322},
        )

        self.assertTrue(matched)
        self.assertEqual(prompt, "ask now")

    def test_strip_required_plain_ask_mentions_does_not_match_role_mentions(self) -> None:
        prompt, matched = mentions.strip_required_plain_ask_mentions(
            "<@&1500506752234422322> ask now",
            {1500506752234422322},
        )

        self.assertFalse(matched)
        self.assertEqual(prompt, "<@&1500506752234422322> ask now")


if __name__ == "__main__":
    _ = unittest.main()
