from __future__ import annotations

import unittest
from dataclasses import dataclass

from codex_discord_mirror_names import (
    get_mirror_project_channel_name,
    get_mirror_thread_name,
)


@dataclass(frozen=True, slots=True)
class Channel:
    id: int
    name: str


@dataclass(frozen=True, slots=True)
class Guild:
    text_channels: list[Channel]


@dataclass(frozen=True, slots=True)
class Thread:
    id: str
    title: str


class MirrorNamesTests(unittest.TestCase):
    def test_project_channel_name_uses_base_name_when_available(self) -> None:
        # Given
        guild = Guild(text_channels=[])

        # When
        result = get_mirror_project_channel_name(
            guild,
            "C:/Repos/My Project",
            "My Project",
        )

        # Then
        self.assertEqual(result, "codex-my-project")

    def test_project_channel_name_adds_digest_when_name_collides(self) -> None:
        # Given
        guild = Guild(text_channels=[Channel(id=1, name="codex-my-project")])

        # When
        result = get_mirror_project_channel_name(
            guild,
            "C:/Repos/My Project",
            "My Project",
        )

        # Then
        self.assertRegex(result, r"^codex-my-project-[0-9a-f]{6}$")

    def test_project_channel_name_ignores_current_channel_collision(self) -> None:
        # Given
        guild = Guild(text_channels=[Channel(id=7, name="codex-my-project")])

        # When
        result = get_mirror_project_channel_name(
            guild,
            "C:/Repos/My Project",
            "My Project",
            current_channel_id=7,
        )

        # Then
        self.assertEqual(result, "codex-my-project")

    def test_thread_name_prefers_ui_name(self) -> None:
        # Given
        thread = Thread(id="abcdef123456", title="Fallback title")

        # When
        result = get_mirror_thread_name(
            thread,
            get_thread_ui_name=lambda _thread_id, _thread: "UI title",
        )

        # Then
        self.assertEqual(result, "UI title")

    def test_thread_name_falls_back_to_thread_title(self) -> None:
        # Given
        thread = Thread(id="abcdef123456", title="Fallback title")

        # When
        result = get_mirror_thread_name(
            thread,
            get_thread_ui_name=lambda _thread_id, _thread: "",
        )

        # Then
        self.assertEqual(result, "Fallback title")
