from __future__ import annotations

from pathlib import Path
from tempfile import TemporaryDirectory
import unittest

import codex_app_server_transport_delivery as app_server_delivery
import codex_discord_prompt_mapped_delivery as mapped_delivery
from codex_thread_models import ThreadInfo
from tests.test_codex_app_server_transport_delivery import (
    FakeBridge,
    FakeDeliveryClient,
)
from tests.test_codex_discord_prompt_mapped_delivery import DepsFixture, FakeChannel


class DeliveryPendingFalseAcceptanceRegressionTests(unittest.IsolatedAsyncioTestCase):
    async def test_acknowledged_steer_without_rollout_record_is_not_accepted(self) -> None:
        with TemporaryDirectory() as temp_dir:
            session_path = Path(temp_dir) / "session.jsonl"
            _ = session_path.write_text("", encoding="utf-8")
            thread = ThreadInfo(
                id="thread-1",
                title="Thread thread-1",
                cwd=str(session_path.parent),
                updated_at=1,
                rollout_path=str(session_path),
                model="gpt",
                reasoning_effort="high",
                tokens_used=0,
            )
            bridge = FakeBridge(thread, delivered_thread=None)
            client = FakeDeliveryClient(active_turn_id="active-turn")

            transport_result = app_server_delivery.steer_or_start_no_wait(
                client,
                "unrecorded prompt",
                thread.id,
                bridge_module=bridge,
                confirm_timeout_sec=1.0,
            )
            fixture = DepsFixture(
                transport_result=(transport_result.exit_code, transport_result.output),
                pending=True,
            )
            mapped_result = await mapped_delivery.handle_mapped_prompt_delivery(
                FakeChannel(),
                "unrecorded prompt",
                thread.id,
                deps=fixture.build(),
            )

        self.assertTrue(transport_result.delivery_pending)
        self.assertEqual(bridge.waited_prompts, ["unrecorded prompt"])
        self.assertEqual(client.steered, [(thread.id, "unrecorded prompt", "active-turn")])
        self.assertEqual(mapped_result.turn_id, "active-turn")
        self.assertFalse(mapped_result.accepted)


if __name__ == "__main__":
    _ = unittest.main()
