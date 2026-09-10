# pyright: reportAssignmentType=false, reportAttributeAccessIssue=false, reportUnknownMemberType=false, reportUnknownVariableType=false
from __future__ import annotations

from contextlib import redirect_stdout
from dataclasses import dataclass
import io
from pathlib import Path
import tempfile
import unittest

import codex_desktop_bridge as bridge
from codex_thread_models import ThreadInfo


@dataclass(frozen=True, slots=True)
class FakeRunner:
    pid: int = 1234

    def poll(self) -> None:
        return None


class BridgeNewDefaultTimeoutTests(unittest.TestCase):
    def test_handles_slow_local_state_persistence(self) -> None:
        original_load_recent_threads = bridge.load_recent_threads
        original_spawn_runner = bridge.spawn_background_new_thread_runner
        original_resolve_cwd = bridge.resolve_new_thread_cwd
        original_wait_delivery = bridge.wait_for_prompt_delivery
        original_set_selected = bridge.set_selected_thread_id
        original_sync_session_index = bridge.sync_session_index_with_state
        original_time = bridge.time.time
        original_sleep = bridge.time.sleep

        selected_ids: list[str | None] = []
        clock_now = [0.0]
        old_thread = ThreadInfo(
            id="old-thread",
            title="old",
            cwd=r"C:\project",
            updated_at=1,
            rollout_path="",
            model="",
            reasoning_effort="",
            tokens_used=0,
        )

        try:
            with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
                session_path = Path(temp_dir) / "session.jsonl"
                _ = session_path.write_text("", encoding="utf-8")
                new_thread = ThreadInfo(
                    id="new-thread",
                    title="new",
                    cwd=str(Path(temp_dir)),
                    updated_at=2,
                    rollout_path=str(session_path),
                    model="",
                    reasoning_effort="",
                    tokens_used=0,
                )

                def fake_load_recent_threads(limit: int = 20) -> list[ThreadInfo]:
                    _ = limit
                    if clock_now[0] >= 9.0:
                        return [new_thread, old_thread]
                    return [old_thread]

                def fake_spawn_runner(prompt: str, cwd: str) -> FakeRunner:
                    _ = (prompt, cwd)
                    return FakeRunner()

                def fake_resolve_cwd(cwd: str | None) -> str:
                    _ = cwd
                    return str(Path(temp_dir))

                def fake_wait_delivery(
                    session_offsets: dict[str, tuple[ThreadInfo, Path, int]],
                    prompt: str,
                    timeout_sec: float = 4.0,
                ) -> ThreadInfo:
                    _ = (session_offsets, prompt, timeout_sec)
                    return new_thread

                def fake_sleep(seconds: float) -> None:
                    clock_now[0] += seconds

                bridge.load_recent_threads = fake_load_recent_threads
                bridge.spawn_background_new_thread_runner = fake_spawn_runner
                bridge.resolve_new_thread_cwd = fake_resolve_cwd
                bridge.wait_for_prompt_delivery = fake_wait_delivery
                bridge.set_selected_thread_id = selected_ids.append
                bridge.sync_session_index_with_state = lambda: 2
                bridge.time.time = lambda: clock_now[0]
                bridge.time.sleep = fake_sleep

                args = bridge.build_parser().parse_args(["new", "--cwd", str(Path(temp_dir)), "start here"])
                self.assertEqual(args.create_timeout, 30.0)

                stdout = io.StringIO()
                with redirect_stdout(stdout):
                    exit_code = bridge.command_new(args)

            self.assertEqual(exit_code, 0)
            self.assertEqual(selected_ids, ["new-thread"])
            self.assertIn("target_thread: new-thread", stdout.getvalue())
        finally:
            bridge.load_recent_threads = original_load_recent_threads
            bridge.spawn_background_new_thread_runner = original_spawn_runner
            bridge.resolve_new_thread_cwd = original_resolve_cwd
            bridge.wait_for_prompt_delivery = original_wait_delivery
            bridge.set_selected_thread_id = original_set_selected
            bridge.sync_session_index_with_state = original_sync_session_index
            bridge.time.time = original_time
            bridge.time.sleep = original_sleep


if __name__ == "__main__":
    _ = unittest.main()
