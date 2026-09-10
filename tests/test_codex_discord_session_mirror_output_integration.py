from __future__ import annotations

from pathlib import Path
from typing import Protocol, cast
import tempfile
import unittest

import codex_desktop_bridge as bridge
import codex_discord_bot as bot
import codex_discord_thread_state as discord_thread_state

from tests.test_codex_discord_ask_busy_failure_integration import run_prompt_and_send
from tests.test_codex_discord_bot import EnvPatch, FakeTarget


class SetSelectedThreadId(Protocol):
    def __call__(self, thread_id: str | None) -> None:
        ...


class GetBusyStateForThread(Protocol):
    def __call__(
        self,
        target_thread_id: str | None,
        **kwargs: str | int | bool | None,
    ) -> tuple[str, str | None, str]:
        ...


class MissingSessionThread:
    def __init__(self, rollout_path: str) -> None:
        self.rollout_path: str = rollout_path


class SessionThreadLike(Protocol):
    rollout_path: str


class ChooseThreadForSession(Protocol):
    def __call__(self, thread_id: str | None = None, cwd: str | None = None) -> SessionThreadLike:
        ...


class DiscordSessionMirrorOutputIntegrationTests(unittest.IsolatedAsyncioTestCase):
    async def test_mapped_ask_uses_transport_no_wait_and_session_mirror_output(self) -> None:
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

            setattr(bridge, "set_selected_thread_id", fake_set_selected_thread_id)

            def fake_run_transport(prompt: str, target_thread_id: str | None) -> tuple[int, str]:
                _ = prompt
                order.append(("transport", target_thread_id))
                return 0, "[transport_delivery] owner_client=client-1 turn_id=turn-1"

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
            self.assertIn("ask_transport_no_wait_done exit=0 target=thread-1", log_text)
            self.assertIn("mapped_prompt_selected_thread_synced target=thread-1", log_text)
            self.assertNotIn("ask_stream_delegated_to_session_mirror target=thread-1", log_text)
        finally:
            bot.get_session_mirror_state().active_output_targets.clear()
            bot.get_session_mirror_state().active_output_targets.update(old_active_targets)
            bot.resolve_target_ref = original_resolve_target_ref
            bot.run_transport_prompt_no_wait = original_run_transport
            bot.get_mirrored_codex_thread_id = original_get_mirrored
            bot.prime_session_mirror_cursor_for_target = original_prime_cursor
            setattr(bridge, "set_selected_thread_id", original_set_selected)

    async def test_mapped_ask_pending_delivery_keeps_session_mirror_output_active(self) -> None:
        original_resolve_target_ref = bot.resolve_target_ref
        original_run_transport = bot.run_transport_prompt_no_wait
        original_get_mirrored = bot.get_mirrored_codex_thread_id
        original_prime_cursor = bot.prime_session_mirror_cursor_for_target
        original_set_selected = cast(SetSelectedThreadId, getattr(bridge, "set_selected_thread_id"))
        old_active_targets = dict(bot.get_session_mirror_state().active_output_targets)
        selected_ids: list[str | None] = []
        try:
            bot.get_session_mirror_state().active_output_targets.clear()
            bot.resolve_target_ref = lambda target_thread_id: (target_thread_id, "taxlab:1")
            bot.get_mirrored_codex_thread_id = lambda channel_id: "thread-1" if channel_id == 222 else None
            bot.prime_session_mirror_cursor_for_target = lambda target_thread_id: 100
            setattr(bridge, "set_selected_thread_id", selected_ids.append)

            def fake_run_transport(prompt: str, target_thread_id: str | None) -> tuple[int, str]:
                _ = prompt, target_thread_id
                return (
                    1,
                    "\n".join(
                        [
                            "target_thread: thread-1",
                            "ui_activation: ipc-thread-follower-start-turn",
                            "ERROR: Prompt delivery could not be confirmed in any recent Codex thread after IPC delivery. The transport reported success, but no matching user message was recorded.",
                        ]
                    ),
                )

            bot.run_transport_prompt_no_wait = fake_run_transport

            channel = FakeTarget(channel_id=222)
            with EnvPatch("DISCORD_SESSION_MIRROR", "1"):
                await run_prompt_and_send()(
                    channel,
                    "please run",
                    ack_sent=True,
                    target_thread_id="thread-1",
                )

            self.assertEqual(len(channel.messages), 1)
            content, view = channel.messages[0]
            self.assertIn("[delivery_pending]", content)
            self.assertNotIn("Ask failed", content)
            self.assertIsNone(view)
            self.assertEqual(selected_ids, ["thread-1"])
            self.assertTrue(bot.is_active_session_mirror_output_target("thread-1"))
        finally:
            bot.get_session_mirror_state().active_output_targets.clear()
            bot.get_session_mirror_state().active_output_targets.update(old_active_targets)
            bot.resolve_target_ref = original_resolve_target_ref
            bot.run_transport_prompt_no_wait = original_run_transport
            bot.get_mirrored_codex_thread_id = original_get_mirrored
            bot.prime_session_mirror_cursor_for_target = original_prime_cursor
            setattr(bridge, "set_selected_thread_id", original_set_selected)

    async def test_active_session_mirror_output_counts_as_thread_runner_busy(self) -> None:
        original_get_busy_state = cast(
            GetBusyStateForThread,
            getattr(discord_thread_state, "get_busy_state_for_thread"),
        )
        old_active_targets = dict(bot.get_session_mirror_state().active_output_targets)
        old_pending_targets = set(bot.get_session_mirror_state().pending_cursor_targets)
        try:
            bot.get_session_mirror_state().active_output_targets.clear()
            bot.get_session_mirror_state().pending_cursor_targets.clear()

            def fake_get_busy_state_for_thread(
                target_thread_id: str | None,
                **kwargs: str | int | bool | None,
            ) -> tuple[str, str | None, str]:
                _ = kwargs
                return "idle", target_thread_id, "taxlab:1"

            setattr(discord_thread_state, "get_busy_state_for_thread", fake_get_busy_state_for_thread)

            self.assertFalse(await bot.is_thread_runner_busy("thread-1"))
            self.assertEqual(bot.get_busy_state_for_thread("thread-1"), ("idle", "thread-1", "taxlab:1"))

            bot.activate_session_mirror_output_target("thread-1")

            self.assertTrue(await bot.is_thread_runner_busy("thread-1"))
            self.assertEqual(bot.get_busy_state_for_thread("thread-1"), ("busy", "thread-1", "taxlab:1"))
        finally:
            setattr(discord_thread_state, "get_busy_state_for_thread", original_get_busy_state)
            bot.get_session_mirror_state().active_output_targets.clear()
            bot.get_session_mirror_state().active_output_targets.update(old_active_targets)
            bot.get_session_mirror_state().pending_cursor_targets.clear()
            bot.get_session_mirror_state().pending_cursor_targets.update(old_pending_targets)

    async def test_mapped_ask_activates_pending_session_mirror_when_session_missing(self) -> None:
        original_resolve_target_ref = bot.resolve_target_ref
        original_run_transport = bot.run_transport_prompt_no_wait
        original_get_mirrored = bot.get_mirrored_codex_thread_id
        original_prime_cursor = bot.prime_session_mirror_cursor_for_target
        original_choose_thread = cast(ChooseThreadForSession, getattr(bridge, "choose_thread"))
        original_set_selected = cast(SetSelectedThreadId, getattr(bridge, "set_selected_thread_id"))
        old_active_targets = dict(bot.get_session_mirror_state().active_output_targets)
        old_pending_targets = set(bot.get_session_mirror_state().pending_cursor_targets)
        order: list[tuple[str, str | None]] = []
        try:
            bot.get_session_mirror_state().active_output_targets.clear()
            bot.get_session_mirror_state().pending_cursor_targets.clear()
            bot.resolve_target_ref = lambda target_thread_id: (target_thread_id, "taxlab:1")
            bot.get_mirrored_codex_thread_id = lambda channel_id: "thread-1" if channel_id == 222 else None
            bot.prime_session_mirror_cursor_for_target = lambda target_thread_id: order.append(
                ("prime", target_thread_id)
            ) or None

            def fake_set_selected_thread_id(thread_id: str | None) -> None:
                order.append(("selected", thread_id))

            def fake_run_transport(prompt: str, target_thread_id: str | None) -> tuple[int, str]:
                _ = prompt
                order.append(("transport", target_thread_id))
                return 0, "[transport_delivery] owner_client=client-1 turn_id=turn-1"

            setattr(bridge, "set_selected_thread_id", fake_set_selected_thread_id)
            bot.run_transport_prompt_no_wait = fake_run_transport

            with tempfile.TemporaryDirectory() as temp_dir:
                missing_session_path = Path(temp_dir) / "new-session.jsonl"

                def fake_choose_thread(
                    thread_id: str | None = None,
                    cwd: str | None = None,
                ) -> MissingSessionThread:
                    _ = thread_id, cwd
                    return MissingSessionThread(str(missing_session_path))

                setattr(bridge, "choose_thread", fake_choose_thread)
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
            self.assertTrue(bot.is_pending_session_mirror_cursor_target("thread-1"))
            self.assertIn("session_mirror_output_pending target=thread-1 reason=session_missing", log_text)
            self.assertNotIn("session_mirror_output_prepare_failed target=thread-1 reason=cursor_unavailable", log_text)
        finally:
            bot.get_session_mirror_state().active_output_targets.clear()
            bot.get_session_mirror_state().active_output_targets.update(old_active_targets)
            bot.get_session_mirror_state().pending_cursor_targets.clear()
            bot.get_session_mirror_state().pending_cursor_targets.update(old_pending_targets)
            bot.resolve_target_ref = original_resolve_target_ref
            bot.run_transport_prompt_no_wait = original_run_transport
            bot.get_mirrored_codex_thread_id = original_get_mirrored
            bot.prime_session_mirror_cursor_for_target = original_prime_cursor
            setattr(bridge, "choose_thread", original_choose_thread)
            setattr(bridge, "set_selected_thread_id", original_set_selected)


if __name__ == "__main__":
    _ = unittest.main()
