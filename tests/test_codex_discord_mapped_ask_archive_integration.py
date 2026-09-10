from __future__ import annotations

from pathlib import Path
from typing import Protocol, cast
import tempfile
import unittest

import codex_desktop_bridge as bridge
import codex_discord_bot as bot

from tests.test_codex_discord_ask_busy_failure_integration import run_prompt_and_send
from tests.test_codex_discord_bot import EnvPatch, FakeTarget


class SetSelectedThreadId(Protocol):
    def __call__(self, thread_id: str | None) -> None:
        ...


class DiscordMappedAskArchiveIntegrationTests(unittest.IsolatedAsyncioTestCase):
    async def test_archive_recommended_mapped_ask_still_uses_session_mirror_output(self) -> None:
        original_resolve_target_ref = bot.resolve_target_ref
        original_run_transport = bot.run_transport_prompt_no_wait
        original_get_mirrored = bot.get_mirrored_codex_thread_id
        original_prime_cursor = bot.prime_session_mirror_cursor_for_target
        original_set_selected = cast(SetSelectedThreadId, getattr(bridge, "set_selected_thread_id"))
        old_active_targets = dict(bot.get_session_mirror_state().active_output_targets)
        order: list[tuple[str, str | None]] = []
        try:
            bot.get_session_mirror_state().active_output_targets.clear()
            bot.resolve_target_ref = lambda target_thread_id: (target_thread_id, "taxlab:1")
            bot.get_mirrored_codex_thread_id = lambda channel_id: "thread-1" if channel_id == 222 else None
            bot.prime_session_mirror_cursor_for_target = lambda target_thread_id: order.append(
                ("prime", target_thread_id)
            ) or 100

            def fake_set_selected_thread_id(thread_id: str | None) -> None:
                order.append(("selected", thread_id))

            def fake_run_transport(prompt: str, target_thread_id: str | None) -> tuple[int, str]:
                _ = prompt
                order.append(("transport", target_thread_id))
                return 0, "[transport_delivery] owner_client=client-1 turn_id=turn-1"

            setattr(bridge, "set_selected_thread_id", fake_set_selected_thread_id)
            bot.run_transport_prompt_no_wait = fake_run_transport

            with tempfile.TemporaryDirectory() as temp_dir:
                log_path = Path(temp_dir) / "discord-smoke.log"
                channel = FakeTarget(channel_id=222)
                with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                    with EnvPatch("DISCORD_SESSION_MIRROR", "1"):
                        await run_prompt_and_send()(
                            channel,
                            "please run",
                            ack_sent=True,
                            target_thread_id="thread-1",
                        )
                log_text = log_path.read_text(encoding="utf-8")

            self.assertEqual(channel.messages, [])
            self.assertEqual(
                order,
                [("prime", "thread-1"), ("selected", "thread-1"), ("transport", "thread-1")],
            )
            self.assertTrue(bot.is_active_session_mirror_output_target("thread-1"))
            self.assertNotIn("session_mirror_delegate_disabled target=thread-1 reason=archive_recommended", log_text)
            self.assertNotIn("ask_stream_delegated_to_session_mirror target=thread-1", log_text)
        finally:
            bot.get_session_mirror_state().active_output_targets.clear()
            bot.get_session_mirror_state().active_output_targets.update(old_active_targets)
            bot.resolve_target_ref = original_resolve_target_ref
            bot.run_transport_prompt_no_wait = original_run_transport
            bot.get_mirrored_codex_thread_id = original_get_mirrored
            bot.prime_session_mirror_cursor_for_target = original_prime_cursor
            setattr(bridge, "set_selected_thread_id", original_set_selected)


if __name__ == "__main__":
    _ = unittest.main()
