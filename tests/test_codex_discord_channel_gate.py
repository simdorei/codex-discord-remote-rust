from __future__ import annotations

from dataclasses import dataclass
import unittest

import codex_discord_channel_gate as channel_gate


@dataclass(frozen=True, slots=True)
class FakeCategory:
    name: str | None


@dataclass(frozen=True, slots=True)
class FakeParent:
    category: FakeCategory | None = None


@dataclass(frozen=True, slots=True)
class FakeChannel:
    id: int | None
    parent_id: int | None = None
    parent: FakeParent | None = None
    category: FakeCategory | None = None


@dataclass(frozen=True, slots=True)
class FakeTextChannelWithoutParentId:
    id: int | None
    parent: FakeParent | None = None
    category: FakeCategory | None = None


class ChannelGateTests(unittest.TestCase):
    def test_allows_direct_allowed_channel_id(self) -> None:
        channel = FakeChannel(id=10)

        result = channel_gate.is_allowed_message_channel(
            channel,
            is_allowed_channel_func=lambda channel_id: channel_id == 10,
            is_mirrored_channel_id_func=lambda channel_id: False,
        )

        self.assertTrue(result)

    def test_allows_direct_allowed_text_channel_without_parent_id(self) -> None:
        channel = FakeTextChannelWithoutParentId(id=10)

        result = channel_gate.is_allowed_message_channel(
            channel,
            is_allowed_channel_func=lambda channel_id: channel_id == 10,
            is_mirrored_channel_id_func=lambda channel_id: False,
        )

        self.assertTrue(result)

    def test_allows_mirrored_parent_channel_id(self) -> None:
        channel = FakeChannel(id=10, parent_id=20)

        result = channel_gate.is_allowed_message_channel(
            channel,
            is_allowed_channel_func=lambda channel_id: False,
            is_mirrored_channel_id_func=lambda channel_id: channel_id == 20,
        )

        self.assertTrue(result)

    def test_allows_codex_category_from_parent(self) -> None:
        channel = FakeChannel(id=10, parent=FakeParent(category=FakeCategory("Codex")))

        result = channel_gate.is_allowed_message_channel(
            channel,
            is_allowed_channel_func=lambda channel_id: False,
            is_mirrored_channel_id_func=lambda channel_id: False,
        )

        self.assertTrue(result)

    def test_rejects_unmatched_channel(self) -> None:
        channel = FakeChannel(id=10, category=FakeCategory("general"))

        result = channel_gate.is_allowed_message_channel(
            channel,
            is_allowed_channel_func=lambda channel_id: False,
            is_mirrored_channel_id_func=lambda channel_id: False,
        )

        self.assertFalse(result)


if __name__ == "__main__":
    _ = unittest.main()
