from __future__ import annotations

from pathlib import Path
from collections.abc import Callable
from typing import Protocol, TypeAlias, cast
import tempfile
import unittest

import codex_app_server_transport as app_server_transport
import codex_desktop_bridge as bridge
import codex_discord_bot as bot
import codex_discord_prompt_busy_result as prompt_busy_result
import codex_discord_steering as discord_steering
from codex_thread_models import ThreadInfo

from tests.test_codex_discord_bot import EnvPatch


AskCall = dict[str, bool | float | str | None]
ResolveTargetRef: TypeAlias = Callable[[str | None], tuple[str | None, str]]
RunAsk: TypeAlias = Callable[..., tuple[int, str]]
RunSteeringPrompt: TypeAlias = Callable[[str, str | None], discord_steering.SteeringPromptResult]


class BotRuntimeAdapter:
    @property
    def resolve_target_ref(self) -> ResolveTargetRef:
        return cast(ResolveTargetRef, getattr(bot, "resolve_target_ref"))

    @resolve_target_ref.setter
    def resolve_target_ref(self, value: ResolveTargetRef) -> None:
        setattr(bot, "resolve_target_ref", value)

    @property
    def run_ask(self) -> RunAsk:
        return cast(RunAsk, getattr(bot, "run_ask"))

    @run_ask.setter
    def run_ask(self, value: RunAsk) -> None:
        setattr(bot, "run_ask", value)

    @property
    def run_steering_prompt(self) -> RunSteeringPrompt:
        return cast(RunSteeringPrompt, getattr(bot, "run_steering_prompt"))

    @property
    def STEERING_DELIVERY_CONFIRM_TIMEOUT_SECONDS(self) -> float:
        return cast(float, getattr(bot, "STEERING_DELIVERY_CONFIRM_TIMEOUT_SECONDS"))


bot_runtime = BotRuntimeAdapter()


class ChooseThread(Protocol):
    def __call__(self, thread_id: str | None, cwd: str | None = None) -> ThreadInfo:
        ...


class SnapshotRecentSessionOffsets(Protocol):
    def __call__(
        self,
        limit: int = 10,
        include_threads: list[ThreadInfo] | None = None,
    ) -> prompt_busy_result.RecentOffsets:
        ...


class WaitForPromptDelivery(Protocol):
    def __call__(
        self,
        session_offsets: prompt_busy_result.RecentOffsets,
        prompt: str,
        timeout_sec: float = 4.0,
    ) -> ThreadInfo | None:
        ...


def make_thread(temp_dir: str, session_path: Path) -> ThreadInfo:
    return ThreadInfo(
        id="thread-1",
        title="title",
        cwd=str(Path(temp_dir)),
        updated_at=1,
        rollout_path=str(session_path),
        model="gpt",
        reasoning_effort="high",
        tokens_used=1,
    )


class DiscordSteeringPromptDeliveryIntegrationTests(unittest.TestCase):
    def test_run_steering_prompt_treats_delayed_ipc_delivery_as_success(self) -> None:
        original_resolve_target_ref = bot_runtime.resolve_target_ref
        original_choose_thread = cast(ChooseThread, getattr(bridge, "choose_thread"))
        original_snapshot = cast(SnapshotRecentSessionOffsets, getattr(bridge, "snapshot_recent_session_offsets"))
        original_run_ask = bot_runtime.run_ask
        original_wait = cast(WaitForPromptDelivery, getattr(bridge, "wait_for_prompt_delivery"))
        try:
            with tempfile.TemporaryDirectory() as temp_dir:
                session_path = Path(temp_dir) / "session.jsonl"
                _ = session_path.write_text("", encoding="utf-8")
                thread = make_thread(temp_dir, session_path)
                recent_offsets: prompt_busy_result.RecentOffsets = {"thread-1": (thread, session_path, 12)}
                waits: list[float] = []

                bot_runtime.resolve_target_ref = lambda target_thread_id: ("thread-1", "taxlab:1")

                def fake_choose_thread(thread_id: str | None, cwd: str | None = None) -> ThreadInfo:
                    _ = thread_id, cwd
                    return thread

                def fake_snapshot_recent_session_offsets(
                    limit: int = 10,
                    include_threads: list[ThreadInfo] | None = None,
                ) -> prompt_busy_result.RecentOffsets:
                    _ = limit, include_threads
                    return recent_offsets

                ask_calls: list[AskCall] = []

                def fake_run_ask(
                    prompt: str,
                    *,
                    force_while_busy: bool = False,
                    wait: bool = True,
                    target_thread_id: str | None = None,
                    timeout_sec: float | None = None,
                ) -> tuple[int, str]:
                    _ = prompt
                    ask_calls.append(
                        {
                            "force_while_busy": force_while_busy,
                            "wait": wait,
                            "target_thread_id": target_thread_id,
                            "timeout_sec": timeout_sec,
                        }
                    )
                    return (
                        1,
                        "ERROR: transport returned a nonzero exit, but the prompt may still be recorded.",
                    )

                def fake_wait(
                    session_offsets: prompt_busy_result.RecentOffsets,
                    prompt: str,
                    timeout_sec: float = 4.0,
                ) -> ThreadInfo:
                    _ = session_offsets, prompt
                    waits.append(timeout_sec)
                    return thread

                setattr(bridge, "choose_thread", fake_choose_thread)
                setattr(bridge, "snapshot_recent_session_offsets", fake_snapshot_recent_session_offsets)
                bot_runtime.run_ask = fake_run_ask
                setattr(bridge, "wait_for_prompt_delivery", fake_wait)

                log_path = Path(temp_dir) / "discord-smoke.log"
                with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                    with EnvPatch("CODEX_DISCORD_APP_SERVER_TRANSPORT", "0"):
                        result = bot_runtime.run_steering_prompt("please steer", "thread-1")

            self.assertEqual(result.exit_code, 0)
            self.assertIn("[delivery_verified]", result.output)
            self.assertEqual(result.target_thread_id, "thread-1")
            self.assertEqual(result.session_path, str(session_path))
            self.assertEqual(result.start_offset, 12)
            self.assertEqual(ask_calls[0]["wait"], False)
            self.assertEqual(ask_calls[0]["force_while_busy"], True)
            self.assertIsNotNone(ask_calls[0]["timeout_sec"])
            self.assertGreaterEqual(waits[-1], bot_runtime.STEERING_DELIVERY_CONFIRM_TIMEOUT_SECONDS)
        finally:
            bot_runtime.resolve_target_ref = original_resolve_target_ref
            setattr(bridge, "choose_thread", original_choose_thread)
            setattr(bridge, "snapshot_recent_session_offsets", original_snapshot)
            bot_runtime.run_ask = original_run_ask
            setattr(bridge, "wait_for_prompt_delivery", original_wait)

    def test_run_steering_prompt_uses_resident_app_server_transport_by_default(self) -> None:
        original_resolve_target_ref = bot_runtime.resolve_target_ref
        original_steer = app_server_transport.steer_or_start_no_wait
        calls: list[tuple[str, str | None, app_server_transport.PersistentCodexAppServer]] = []
        try:
            bot_runtime.resolve_target_ref = lambda target_thread_id: ("thread-1", "taxlab:1")

            def fake_steer(
                client: app_server_transport.PersistentCodexAppServer,
                prompt: str,
                target_thread_id: str | None,
                **kwargs: bool | float | str | None,
            ) -> app_server_transport.AppServerDeliveryResult:
                _ = kwargs
                calls.append((prompt, target_thread_id, client))
                return app_server_transport.AppServerDeliveryResult(
                    0,
                    "transport: resident-app-server turn/steer",
                    thread_id="thread-1",
                    turn_id="turn-1",
                    target_ref="taxlab:1",
                    session_path="session.jsonl",
                    start_offset=99,
                    delivery_pending=True,
                )

            app_server_transport.steer_or_start_no_wait = fake_steer

            with EnvPatch("CODEX_DISCORD_APP_SERVER_TRANSPORT", "1"):
                result = bot_runtime.run_steering_prompt("please steer", "thread-1")

            self.assertEqual(result.exit_code, 0)
            self.assertIn("resident-app-server", result.output)
            self.assertEqual(result.target_thread_id, "thread-1")
            self.assertEqual(result.target_ref, "taxlab:1")
            self.assertEqual(result.session_path, "session.jsonl")
            self.assertEqual(result.start_offset, 99)
            self.assertTrue(result.delivery_pending)
            self.assertEqual(calls, [("please steer", "thread-1", app_server_transport.DEFAULT_CLIENT)])
        finally:
            bot_runtime.resolve_target_ref = original_resolve_target_ref
            app_server_transport.steer_or_start_no_wait = original_steer

    def test_run_steering_prompt_surfaces_rollout_thread_id_parse_error(self) -> None:
        original_resolve_target_ref = bot_runtime.resolve_target_ref
        original_steer = app_server_transport.steer_or_start_no_wait
        original_choose_thread = cast(ChooseThread, getattr(bridge, "choose_thread"))
        original_snapshot = cast(SnapshotRecentSessionOffsets, getattr(bridge, "snapshot_recent_session_offsets"))
        original_run_ask = bot_runtime.run_ask
        original_wait = cast(WaitForPromptDelivery, getattr(bridge, "wait_for_prompt_delivery"))
        try:
            with tempfile.TemporaryDirectory() as temp_dir:
                session_path = Path(temp_dir) / "session.jsonl"
                _ = session_path.write_text("", encoding="utf-8")
                thread = make_thread(temp_dir, session_path)
                recent_offsets: prompt_busy_result.RecentOffsets = {"thread-1": (thread, session_path, 12)}
                waits: list[float] = []

                bot_runtime.resolve_target_ref = lambda target_thread_id: ("thread-1", "taxlab:1")

                def fake_steer(
                    client: app_server_transport.PersistentCodexAppServer,
                    prompt: str,
                    target_thread_id: str | None,
                    **kwargs: bool | float | str | None,
                ) -> app_server_transport.AppServerDeliveryResult:
                    _ = client, prompt, target_thread_id, kwargs
                    raise RuntimeError(
                        "thread/resume failed: failed to load rollout "
                        "C:\\Users\\example\\.codex\\sessions\\2026\\07\\04\\rollout.jsonl: "
                        "failed to parse thread ID from rollout file"
                    )

                def fake_choose_thread(thread_id: str | None, cwd: str | None = None) -> ThreadInfo:
                    _ = thread_id, cwd
                    return thread

                def fake_snapshot_recent_session_offsets(
                    limit: int = 10,
                    include_threads: list[ThreadInfo] | None = None,
                ) -> prompt_busy_result.RecentOffsets:
                    _ = limit, include_threads
                    return recent_offsets

                ask_calls: list[AskCall] = []

                def fake_run_ask(
                    prompt: str,
                    *,
                    force_while_busy: bool = False,
                    wait: bool = True,
                    target_thread_id: str | None = None,
                    timeout_sec: float | None = None,
                ) -> tuple[int, str]:
                    ask_calls.append(
                        {
                            "force_while_busy": force_while_busy,
                            "wait": wait,
                            "target_thread_id": target_thread_id,
                            "timeout_sec": timeout_sec,
                        }
                    )
                    raise AssertionError(f"must not open IPC fallback: {prompt}")

                def fake_wait(
                    session_offsets: prompt_busy_result.RecentOffsets,
                    prompt: str,
                    timeout_sec: float = 4.0,
                ) -> ThreadInfo:
                    _ = session_offsets, prompt
                    waits.append(timeout_sec)
                    return thread

                app_server_transport.steer_or_start_no_wait = fake_steer
                setattr(bridge, "choose_thread", fake_choose_thread)
                setattr(bridge, "snapshot_recent_session_offsets", fake_snapshot_recent_session_offsets)
                bot_runtime.run_ask = fake_run_ask
                setattr(bridge, "wait_for_prompt_delivery", fake_wait)

                log_path = Path(temp_dir) / "discord-smoke.log"
                with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                    with EnvPatch("CODEX_DISCORD_APP_SERVER_TRANSPORT", "1"):
                        result = bot_runtime.run_steering_prompt("please steer", "thread-1")
                log_text = log_path.read_text(encoding="utf-8")

            self.assertEqual(result.exit_code, 1)
            self.assertIn("ERROR: resident app-server transport failed:", result.output)
            self.assertIn("failed to parse thread ID from rollout file", result.output)
            self.assertEqual(result.target_thread_id, "thread-1")
            self.assertEqual(ask_calls, [])
            self.assertEqual(waits, [])
            self.assertIn("app_server_steering_failed target=thread-1", log_text)
        finally:
            bot_runtime.resolve_target_ref = original_resolve_target_ref
            app_server_transport.steer_or_start_no_wait = original_steer
            setattr(bridge, "choose_thread", original_choose_thread)
            setattr(bridge, "snapshot_recent_session_offsets", original_snapshot)
            bot_runtime.run_ask = original_run_ask
            setattr(bridge, "wait_for_prompt_delivery", original_wait)

    def test_run_steering_prompt_keeps_watching_pending_ipc_delivery(self) -> None:
        original_resolve_target_ref = bot_runtime.resolve_target_ref
        original_choose_thread = cast(ChooseThread, getattr(bridge, "choose_thread"))
        original_snapshot = cast(SnapshotRecentSessionOffsets, getattr(bridge, "snapshot_recent_session_offsets"))
        original_run_ask = bot_runtime.run_ask
        original_wait = cast(WaitForPromptDelivery, getattr(bridge, "wait_for_prompt_delivery"))
        try:
            with tempfile.TemporaryDirectory() as temp_dir:
                session_path = Path(temp_dir) / "session.jsonl"
                _ = session_path.write_text("", encoding="utf-8")
                thread = make_thread(temp_dir, session_path)
                recent_offsets: prompt_busy_result.RecentOffsets = {"thread-1": (thread, session_path, 12)}
                waits: list[float] = []

                bot_runtime.resolve_target_ref = lambda target_thread_id: ("thread-1", "taxlab:1")

                def fake_choose_thread(thread_id: str | None, cwd: str | None = None) -> ThreadInfo:
                    _ = thread_id, cwd
                    return thread

                def fake_snapshot_recent_session_offsets(
                    limit: int = 10,
                    include_threads: list[ThreadInfo] | None = None,
                ) -> prompt_busy_result.RecentOffsets:
                    _ = limit, include_threads
                    return recent_offsets

                def fake_run_ask(
                    prompt: str,
                    *,
                    force_while_busy: bool = False,
                    wait: bool = True,
                    target_thread_id: str | None = None,
                    timeout_sec: float | None = None,
                ) -> tuple[int, str]:
                    _ = prompt, force_while_busy, wait, target_thread_id, timeout_sec
                    return (
                        1,
                        "target_thread: thread-1\n"
                        + "ui_activation: ipc-thread-follower-start-turn\n"
                        + "ERROR: Prompt delivery could not be confirmed in any recent Codex thread after IPC delivery. "
                        + "The transport reported success, but no matching user message was recorded.",
                    )

                def fake_wait(
                    session_offsets: prompt_busy_result.RecentOffsets,
                    prompt: str,
                    timeout_sec: float = 4.0,
                ) -> None:
                    _ = session_offsets, prompt
                    waits.append(timeout_sec)

                setattr(bridge, "choose_thread", fake_choose_thread)
                setattr(bridge, "snapshot_recent_session_offsets", fake_snapshot_recent_session_offsets)
                bot_runtime.run_ask = fake_run_ask
                setattr(bridge, "wait_for_prompt_delivery", fake_wait)

                log_path = Path(temp_dir) / "discord-smoke.log"
                with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                    with EnvPatch("CODEX_DISCORD_APP_SERVER_TRANSPORT", "0"):
                        result = bot_runtime.run_steering_prompt("please steer", "thread-1")
                log_text = log_path.read_text(encoding="utf-8")

                self.assertEqual(result.exit_code, 0)
                self.assertIn("[delivery_pending]", result.output)
                self.assertNotIn("ERROR:", result.output)
                self.assertNotIn("Prompt delivery could not be confirmed", result.output)
                self.assertEqual(result.target_thread_id, "thread-1")
                self.assertEqual(result.session_path, str(session_path))
                self.assertEqual(result.start_offset, 12)
                self.assertEqual(waits, [])
                self.assertIn("steering_ipc_delivery_pending exit=1 target=thread-1 confirm_timeout=0.0", log_text)
        finally:
            bot_runtime.resolve_target_ref = original_resolve_target_ref
            setattr(bridge, "choose_thread", original_choose_thread)
            setattr(bridge, "snapshot_recent_session_offsets", original_snapshot)
            bot_runtime.run_ask = original_run_ask
            setattr(bridge, "wait_for_prompt_delivery", original_wait)


if __name__ == "__main__":
    _ = unittest.main()
