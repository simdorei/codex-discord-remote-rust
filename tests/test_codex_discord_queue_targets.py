from __future__ import annotations

from dataclasses import dataclass
import unittest
from unittest import mock

import codex_desktop_bridge as bridge
import codex_discord_bot as bot
import codex_discord_queue_targets as queue_targets


@dataclass(frozen=True, slots=True)
class FakeThread:
    id: str


class FakeBridge:
    def __init__(self) -> None:
        self.refs: list[str] = []

    def resolve_thread_ref(self, ref: str) -> FakeThread:
        self.refs.append(ref)
        return FakeThread(f"resolved-{ref}")


class QueueTargetTests(unittest.TestCase):
    def test_selected_ref_uses_selected_target(self) -> None:
        bridge = FakeBridge()

        self.assertEqual(
            queue_targets.resolve_queue_command_target(
                222,
                " selected ",
                bridge_module=bridge,
                resolve_target_ref_func=lambda thread_id: (thread_id, f"ref:{thread_id}"),
                get_mirrored_codex_thread_id_func=lambda channel_id: "mapped-thread",
            ),
            (None, "selected"),
        )
        self.assertEqual(bridge.refs, [])

    def test_explicit_ref_resolves_through_bridge(self) -> None:
        bridge = FakeBridge()

        self.assertEqual(
            queue_targets.resolve_queue_command_target(
                222,
                "taxlab:7",
                bridge_module=bridge,
                resolve_target_ref_func=lambda thread_id: (thread_id, f"pretty:{thread_id}"),
                get_mirrored_codex_thread_id_func=lambda channel_id: None,
            ),
            ("resolved-taxlab:7", "pretty:resolved-taxlab:7"),
        )
        self.assertEqual(bridge.refs, ["taxlab:7"])

    def test_bot_wrapper_resolves_explicit_ref(self) -> None:
        def fake_resolve_target_ref(thread_id: str) -> tuple[str, str]:
            return thread_id, f"pretty:{thread_id}"

        with (
            mock.patch.object(bridge, "resolve_thread_ref", return_value=FakeThread("resolved-taxlab:7")),
            mock.patch.object(bot, "resolve_target_ref", side_effect=fake_resolve_target_ref),
        ):
            self.assertEqual(
                bot.resolve_queue_command_target(222, "taxlab:7"),
                ("resolved-taxlab:7", "pretty:resolved-taxlab:7"),
            )

    def test_mapped_channel_target_falls_back_to_thread_id_when_ref_missing(self) -> None:
        self.assertEqual(
            queue_targets.resolve_queue_command_target(
                222,
                None,
                bridge_module=FakeBridge(),
                resolve_target_ref_func=lambda thread_id: (thread_id, ""),
                get_mirrored_codex_thread_id_func=lambda channel_id: "mapped-thread",
            ),
            ("mapped-thread", "mapped-thread"),
        )

    def test_unmapped_channel_uses_selected_target(self) -> None:
        self.assertEqual(
            queue_targets.resolve_queue_command_target(
                222,
                "",
                bridge_module=FakeBridge(),
                resolve_target_ref_func=lambda thread_id: (thread_id, f"ref:{thread_id}"),
                get_mirrored_codex_thread_id_func=lambda channel_id: None,
            ),
            (None, "selected"),
        )


if __name__ == "__main__":
    _ = unittest.main()
