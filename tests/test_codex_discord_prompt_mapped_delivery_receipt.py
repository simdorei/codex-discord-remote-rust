from __future__ import annotations

import unittest

import codex_discord_prompt_mapped_delivery as mapped_delivery
from tests.test_codex_discord_prompt_mapped_delivery import DepsFixture, FakeChannel


class MappedPromptDeliveryReceiptTests(unittest.IsolatedAsyncioTestCase):
    async def test_success_preserves_accepted_turn_receipt(self) -> None:
        fixture = DepsFixture(
            transport_result=(
                0,
                "delivered\n[app_server_delivery] turn_id=turn-42",
            )
        )
        channel = FakeChannel()

        result = await mapped_delivery.handle_mapped_prompt_delivery(
            channel,
            "please run",
            target_thread_id="thread-1",
            deps=fixture.build(),
        )

        self.assertTrue(result.handled)
        self.assertTrue(result.accepted)
        self.assertEqual(result.turn_id, "turn-42")
        self.assertEqual(fixture.transport_calls, [("please run", "thread-1")])
        self.assertEqual(channel.typing_events, ["enter", "exit"])
        self.assertEqual(channel.messages, [])

    async def test_desktop_ipc_success_preserves_accepted_turn_receipt(self) -> None:
        fixture = DepsFixture(
            transport_result=(
                0,
                "[ipc_delivery] owner_client=desktop-1 turn_id=turn-ipc",
            )
        )
        channel = FakeChannel()

        result = await mapped_delivery.handle_mapped_prompt_delivery(
            channel,
            "please run",
            target_thread_id="thread-1",
            deps=fixture.build(),
        )

        self.assertTrue(result.accepted)
        self.assertEqual(result.turn_id, "turn-ipc")


if __name__ == "__main__":
    _ = unittest.main()
