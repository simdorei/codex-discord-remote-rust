from __future__ import annotations

import asyncio
import os
import tempfile
import time
import unittest
from pathlib import Path

import codex_discord_runtime as discord_runtime
import codex_discord_runtime_state_bridge as runtime_state_bridge
import codex_discord_session_mirror as discord_session_mirror


class RuntimeStateBridgeTests(unittest.TestCase):
    def test_exit_bot_process_continues_after_unexpected_close_failure(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            lock_path = Path(temp_dir) / "runtime.lock"
            _ = lock_path.write_text(str(os.getpid()), encoding="ascii")
            logs: list[str] = []
            exit_codes: list[int] = []

            bridge = runtime_state_bridge.RuntimeStateBridge(
                session_mirror_state=discord_session_mirror.SessionMirrorState(),
                runtime_state=discord_runtime.DiscordRuntimeState(),
                thread_runners={},
                thread_runners_lock=asyncio.Lock(),
                active_output_ttl_seconds=60.0,
                runtime_mutex_name="unit",
                get_runtime_lock_path=lambda: lock_path,
                log=logs.append,
                close_remote_bridge=lambda: None,
                close_app_server=lambda: (_ for _ in ()).throw(
                    RuntimeError("unexpected close failure")
                ),
                exit_process=exit_codes.append,
            )

            bridge.exit_bot_process(9, reason="unit-close-failure")

            self.assertEqual(exit_codes, [9])
            self.assertFalse(lock_path.exists())
            self.assertTrue(
                any(
                    "bot_process_app_server_close_failed" in line
                    and "unexpected close failure" in line
                    for line in logs
                )
            )

    def test_exit_bot_process_logs_removes_lock_and_exits(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            lock_path = Path(temp_dir) / "runtime.lock"
            _ = lock_path.write_text(str(os.getpid()), encoding="ascii")
            logs: list[str] = []
            exit_codes: list[int] = []
            shutdown_order: list[str] = []

            def exit_process(code: int) -> None:
                shutdown_order.append("process")
                exit_codes.append(code)

            bridge = runtime_state_bridge.RuntimeStateBridge(
                session_mirror_state=discord_session_mirror.SessionMirrorState(),
                runtime_state=discord_runtime.DiscordRuntimeState(),
                thread_runners={},
                thread_runners_lock=asyncio.Lock(),
                active_output_ttl_seconds=60.0,
                runtime_mutex_name="unit",
                get_runtime_lock_path=lambda: lock_path,
                log=logs.append,
                close_remote_bridge=lambda: shutdown_order.append("remote-bridge"),
                close_app_server=lambda: shutdown_order.append("app-server"),
                exit_process=exit_process,
            )

            bridge.exit_bot_process(7, reason="unit")

            self.assertEqual(exit_codes, [7])
            self.assertEqual(
                shutdown_order,
                ["remote-bridge", "app-server", "process"],
            )
            self.assertFalse(lock_path.exists())
            self.assertTrue(
                any("bot_process_exit_requested reason=unit code=7" in line for line in logs)
            )
            self.assertTrue(any("runtime_lock_removed" in line for line in logs))


class RuntimeStateBridgeBusyTests(unittest.IsolatedAsyncioTestCase):
    def make_bridge(
        self,
    ) -> tuple[
        runtime_state_bridge.RuntimeStateBridge,
        discord_session_mirror.SessionMirrorState,
        discord_runtime.DiscordRuntimeState,
        dict[str, discord_runtime.RunnerState],
    ]:
        session_mirror_state = discord_session_mirror.SessionMirrorState()
        runtime_state = discord_runtime.DiscordRuntimeState()
        thread_runners: dict[str, discord_runtime.RunnerState] = {}
        bridge = runtime_state_bridge.RuntimeStateBridge(
            session_mirror_state=session_mirror_state,
            runtime_state=runtime_state,
            thread_runners=thread_runners,
            thread_runners_lock=asyncio.Lock(),
            active_output_ttl_seconds=60.0,
            runtime_mutex_name="unit",
            get_runtime_lock_path=lambda: Path("runtime.lock"),
            log=lambda _line: None,
        )
        return bridge, session_mirror_state, runtime_state, thread_runners

    async def test_direct_ask_claim_release_controls_busy_state(self) -> None:
        bridge, _session_mirror_state, _runtime_state, _thread_runners = self.make_bridge()

        self.assertFalse(await bridge.is_thread_runner_busy("thread-1"))
        self.assertTrue(await bridge.claim_direct_ask_target("thread-1"))
        self.assertFalse(await bridge.claim_direct_ask_target("thread-1"))
        self.assertTrue(await bridge.is_thread_runner_busy("thread-1"))

        await bridge.release_direct_ask_target("thread-1")

        self.assertFalse(await bridge.is_thread_runner_busy("thread-1"))

    async def test_ask_delivery_lock_is_reused_and_controls_busy_state(self) -> None:
        bridge, _session_mirror_state, _runtime_state, _thread_runners = self.make_bridge()

        lock = bridge.get_ask_delivery_lock("thread-1")

        self.assertIs(lock, bridge.get_ask_delivery_lock("thread-1"))
        self.assertFalse(await bridge.is_thread_runner_busy("thread-1"))
        async with lock:
            self.assertTrue(await bridge.is_thread_runner_busy("thread-1"))

    async def test_runner_queue_and_session_mirror_output_mark_busy(self) -> None:
        bridge, session_mirror_state, _runtime_state, thread_runners = self.make_bridge()
        queue: asyncio.Queue[object] = asyncio.Queue()
        await queue.put("queued")
        thread_runners["thread-1"] = {
            "queue": queue,
            "active": False,
            "target_thread_id": "thread-1",
        }
        session_mirror_state.active_output_targets["thread-2"] = time.monotonic()

        self.assertTrue(await bridge.is_thread_runner_busy("thread-1"))
        self.assertTrue(await bridge.is_thread_runner_busy("thread-2"))


if __name__ == "__main__":
    _ = unittest.main()
