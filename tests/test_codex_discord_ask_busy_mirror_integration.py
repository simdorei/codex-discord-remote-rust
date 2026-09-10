from __future__ import annotations

from pathlib import Path
from typing import cast
import tempfile
import unittest

import codex_desktop_bridge as bridge
import codex_discord_bot as bot
from codex_thread_models import ThreadInfo

from tests.test_codex_discord_ask_busy_failure_integration import (
    ChooseThread,
    SessionOffsetMap,
    SnapshotRecentSessionOffsets,
    WaitForPromptDelivery,
    make_thread,
    run_prompt_and_send,
)
from tests.test_codex_discord_bot import EnvPatch, FakeMessage


class DiscordAskBusyMirrorIntegrationTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self) -> None:
        original_app_server_transport_enabled = bot.app_server_transport_enabled

        def restore_app_server_transport_enabled() -> None:
            bot.app_server_transport_enabled = original_app_server_transport_enabled

        self.addCleanup(restore_app_server_transport_enabled)
        bot.app_server_transport_enabled = lambda: False

    async def test_ask_target_busy_failure_in_mirrored_thread_waits_for_session_mirror(self) -> None:
        original_resolve_target_ref = bot.resolve_target_ref
        original_run_ask_stream = bot.run_ask_stream
        original_build_context_warning = bot.build_context_warning
        original_get_interactive_state = bot.get_interactive_state_for_thread
        original_should_delegate = bot.should_delegate_output_to_session_mirror
        original_wait_settle = bot.wait_for_mirrored_busy_delegation_settle
        original_choose_thread = cast(ChooseThread, getattr(bridge, "choose_thread"))
        original_snapshot = cast(SnapshotRecentSessionOffsets, getattr(bridge, "snapshot_recent_session_offsets"))
        original_wait = cast(WaitForPromptDelivery, getattr(bridge, "wait_for_prompt_delivery"))
        calls: list[bool] = []
        settle_calls: list[tuple[str, str | None]] = []
        try:
            with tempfile.TemporaryDirectory() as temp_dir:
                session_path = Path(temp_dir) / "session.jsonl"
                _ = session_path.write_text("", encoding="utf-8")
                thread = make_thread(temp_dir, session_path)
                recent_offsets: SessionOffsetMap = {"thread-1": (thread, session_path, 0)}

                bot.resolve_target_ref = lambda target_thread_id: (target_thread_id, "taxlab:1")
                bot.get_interactive_state_for_thread = lambda target_thread_id: ("", None, "")
                bot.should_delegate_output_to_session_mirror = lambda channel, target_thread_id: True

                def fake_choose_thread(
                    thread_id: str | None = None,
                    cwd: str | None = None,
                ) -> ThreadInfo:
                    _ = thread_id, cwd
                    return thread

                def fake_snapshot_recent_session_offsets(
                    limit: int = 10,
                    include_threads: set[str] | None = None,
                ) -> SessionOffsetMap:
                    _ = limit, include_threads
                    return recent_offsets

                def fake_wait_for_prompt_delivery(
                    session_offsets: SessionOffsetMap,
                    prompt: str,
                    timeout_sec: float = 4.0,
                ) -> None:
                    _ = session_offsets, prompt, timeout_sec

                async def fake_wait_settle(
                    prompt: str,
                    *,
                    target_thread_id: str | None = None,
                    recent_offsets: SessionOffsetMap | None = None,
                ) -> None:
                    _ = recent_offsets
                    settle_calls.append((prompt, target_thread_id))

                def fake_run_ask_stream(
                    prompt: str,
                    relay: bot.DiscordAskRelay,
                    *,
                    force_while_busy: bool = False,
                    wait: bool = True,
                    target_thread_id: str | None = None,
                ) -> tuple[int, str]:
                    _ = prompt, relay, wait, target_thread_id
                    calls.append(force_while_busy)
                    return (
                        1,
                        "\n".join(
                            [
                                "Ask failed (exit 1)",
                                "",
                                "ERROR: The selected thread is still busy. Wait, switch to another thread, or pass --force-while-busy.",
                            ]
                        ),
                    )

                setattr(bridge, "choose_thread", fake_choose_thread)
                setattr(bridge, "snapshot_recent_session_offsets", fake_snapshot_recent_session_offsets)
                setattr(bridge, "wait_for_prompt_delivery", fake_wait_for_prompt_delivery)
                bot.wait_for_mirrored_busy_delegation_settle = fake_wait_settle
                bot.run_ask_stream = fake_run_ask_stream
                bot.build_context_warning = lambda target_thread_id: ""

                log_path = Path(temp_dir) / "discord-smoke.log"
                message = FakeMessage()
                with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                    with EnvPatch("DISCORD_ASK_BUSY_RETRY_ATTEMPTS", "0"):
                        await run_prompt_and_send()(
                            message.channel,
                            "please steer",
                            ack_sent=True,
                            source_message=message,
                            target_thread_id="thread-1",
                        )
                log_text = log_path.read_text(encoding="utf-8")

            self.assertEqual(message.channel.messages, [])
            self.assertEqual(calls, [False])
            self.assertEqual(settle_calls, [("please steer", "thread-1")])
            self.assertIn("ask_stream_busy_transport_failure kind=target target=thread-1", log_text)
            self.assertIn("ask_stream_busy_delegated_to_session_mirror target=thread-1", log_text)
            self.assertNotIn("ask_stream_busy_retry_exhausted", log_text)
        finally:
            bot.resolve_target_ref = original_resolve_target_ref
            bot.run_ask_stream = original_run_ask_stream
            bot.build_context_warning = original_build_context_warning
            bot.get_interactive_state_for_thread = original_get_interactive_state
            bot.should_delegate_output_to_session_mirror = original_should_delegate
            bot.wait_for_mirrored_busy_delegation_settle = original_wait_settle
            setattr(bridge, "choose_thread", original_choose_thread)
            setattr(bridge, "snapshot_recent_session_offsets", original_snapshot)
            setattr(bridge, "wait_for_prompt_delivery", original_wait)


if __name__ == "__main__":
    _ = unittest.main()
