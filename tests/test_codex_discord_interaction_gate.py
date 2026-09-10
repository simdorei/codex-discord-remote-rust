from __future__ import annotations

from dataclasses import dataclass
import os
import unittest
from unittest.mock import patch

import codex_discord_interaction_gate as gate


@dataclass(frozen=True, slots=True)
class FakeUser:
    id: int


@dataclass(frozen=True, slots=True)
class FakeChannel:
    id: int


@dataclass(frozen=True, slots=True)
class FakeInteraction:
    user: FakeUser | None
    channel_id: int
    channel: FakeChannel | None = None


class FakeBot:
    def __init__(
        self,
        *,
        allowed_users: set[int] | None = None,
        allowed_channels: set[int] | None = None,
        allowed_message_channels: set[int] | None = None,
    ) -> None:
        self.allowed_users: set[int] = allowed_users or {7}
        self.allowed_channels: set[int] = allowed_channels or {11}
        self.allowed_message_channels: set[int] = allowed_message_channels or set()

    def is_allowed_user(self, user_id: object) -> bool:
        return user_id in self.allowed_users

    def is_allowed_channel(self, channel_id: object) -> bool:
        return channel_id in self.allowed_channels

    def is_allowed_message_channel(self, channel: object) -> bool:
        return getattr(channel, "id", None) in self.allowed_message_channels


def command_name(_interaction: object) -> str:
    return "ask"


class InteractionGateTests(unittest.TestCase):
    def test_discord_user_allowlist_uses_env_when_present(self) -> None:
        with patch.dict(os.environ, {}, clear=True):
            self.assertTrue(gate.is_discord_user_allowed(None))
            self.assertTrue(gate.is_discord_user_allowed(7))

        with patch.dict(os.environ, {"DISCORD_ALLOWED_USER_IDS": "7, bad, 8"}, clear=True):
            self.assertTrue(gate.is_discord_user_allowed(7))
            self.assertTrue(gate.is_discord_user_allowed(8))
            self.assertFalse(gate.is_discord_user_allowed(9))
            self.assertFalse(gate.is_discord_user_allowed(None))

    def test_rejects_disallowed_user_and_logs_reason(self) -> None:
        logs: list[str] = []
        interaction = FakeInteraction(user=FakeUser(id=8), channel_id=11)

        allowed = gate.check_interaction_allowed(
            FakeBot(),
            interaction,
            log_func=logs.append,
            get_interaction_command_name_func=command_name,
            is_mirrored_channel_id_func=lambda _channel_id: False,
        )

        self.assertFalse(allowed)
        self.assertEqual(
            logs,
            ["slash_ignored command=ask reason=user_not_allowed user=8 channel=11"],
        )

    def test_allows_explicit_allowed_channel(self) -> None:
        logs: list[str] = []
        interaction = FakeInteraction(user=FakeUser(id=7), channel_id=11)

        allowed = gate.check_interaction_allowed(
            FakeBot(),
            interaction,
            log_func=logs.append,
            get_interaction_command_name_func=command_name,
            is_mirrored_channel_id_func=lambda _channel_id: False,
        )

        self.assertTrue(allowed)
        self.assertEqual(logs, [])

    def test_allows_mirrored_channel(self) -> None:
        logs: list[str] = []
        interaction = FakeInteraction(user=FakeUser(id=7), channel_id=22)

        allowed = gate.check_interaction_allowed(
            FakeBot(),
            interaction,
            log_func=logs.append,
            get_interaction_command_name_func=command_name,
            is_mirrored_channel_id_func=lambda channel_id: channel_id == 22,
        )

        self.assertTrue(allowed)
        self.assertEqual(logs, [])

    def test_allows_allowed_message_channel_object(self) -> None:
        logs: list[str] = []
        channel = FakeChannel(id=33)
        interaction = FakeInteraction(user=FakeUser(id=7), channel_id=33, channel=channel)

        allowed = gate.check_interaction_allowed(
            FakeBot(allowed_message_channels={33}),
            interaction,
            log_func=logs.append,
            get_interaction_command_name_func=command_name,
            is_mirrored_channel_id_func=lambda _channel_id: False,
        )

        self.assertTrue(allowed)
        self.assertEqual(logs, [])

    def test_rejects_disallowed_channel_and_logs_reason(self) -> None:
        logs: list[str] = []
        interaction = FakeInteraction(user=FakeUser(id=7), channel_id=44)

        allowed = gate.check_interaction_allowed(
            FakeBot(),
            interaction,
            log_func=logs.append,
            get_interaction_command_name_func=command_name,
            is_mirrored_channel_id_func=lambda _channel_id: False,
        )

        self.assertFalse(allowed)
        self.assertEqual(
            logs,
            ["slash_ignored command=ask reason=channel_not_allowed user=7 channel=44"],
        )
