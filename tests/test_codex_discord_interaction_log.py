from __future__ import annotations

import unittest
from dataclasses import dataclass

import codex_discord_interaction_log as interaction_log


@dataclass(frozen=True, slots=True)
class NamedType:
    name: str


@dataclass(frozen=True, slots=True)
class FakeInteraction:
    type: NamedType | str | None


@dataclass(frozen=True, slots=True)
class FakeUser:
    id: int | str | None


@dataclass(frozen=True, slots=True)
class FakeDataInteraction:
    data: interaction_log.RawInteractionData | None


class DiscordInteractionLogTests(unittest.TestCase):
    def test_format_interaction_type_prefers_type_name(self) -> None:
        self.assertEqual(
            interaction_log.format_interaction_type(FakeInteraction(NamedType("component"))),
            "component",
        )

    def test_format_raw_interaction_command_prefers_name_then_custom_id(self) -> None:
        self.assertEqual(
            interaction_log.format_raw_interaction_command({"data": {"name": "ask"}}),
            "ask",
        )
        self.assertEqual(
            interaction_log.format_raw_interaction_command({"data": {"custom_id": "busy:steer"}}),
            "busy:steer",
        )

    def test_format_discord_user_id_for_log_uses_dash_for_missing_id(self) -> None:
        self.assertEqual(interaction_log.format_discord_user_id_for_log(FakeUser(123)), "123")
        self.assertEqual(interaction_log.format_discord_user_id_for_log(FakeUser(None)), "-")
        self.assertEqual(interaction_log.format_discord_user_id_for_log("fake-user"), "-")

    def test_get_interaction_custom_id_formats_present_custom_id(self) -> None:
        self.assertEqual(
            interaction_log.get_interaction_custom_id(
                FakeDataInteraction({"custom_id": "busy:steer"})
            ),
            "busy:steer",
        )
        self.assertEqual(interaction_log.get_interaction_custom_id(FakeDataInteraction(None)), "-")
