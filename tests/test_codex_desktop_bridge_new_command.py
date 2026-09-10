from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
import tempfile
import unittest

import codex_desktop_bridge_new_command as new_command
from codex_thread_models import ThreadInfo


@dataclass(frozen=True, slots=True)
class FakeRunner:
    exit_code: int | None = None
    pid: int = 4321

    def poll(self) -> int | None:
        return self.exit_code


@dataclass(frozen=True, slots=True)
class NewCommandHarness:
    deps: new_command.NewCommandDeps
    events: list[str]
    runner: FakeRunner


class NewCommandTests(unittest.TestCase):
    def test_missing_prompt_raises_typed_error_without_spawning(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            harness = _make_harness(Path(temp_dir), created_thread=None)

            with self.assertRaises(new_command.NewCommandMissingPromptError) as raised:
                new_command.run_new_command(
                    cwd=None,
                    prompt="",
                    abort=False,
                    create_timeout=0.1,
                    deps=harness.deps,
                )

        self.assertIn("Background `new` requires an initial prompt.", str(raised.exception))
        self.assertNotIn("spawn:", "\n".join(harness.events))

    def test_running_runner_timeout_raises_typed_error(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            harness = _make_harness(Path(temp_dir), created_thread=None)

            with self.assertRaises(new_command.NewCommandThreadTimeoutError) as raised:
                new_command.run_new_command(
                    cwd=None,
                    prompt="hello",
                    abort=False,
                    create_timeout=0.1,
                    deps=harness.deps,
                )

        self.assertIn("started, but a new persisted thread did not appear", str(raised.exception))

    def test_exited_runner_timeout_raises_typed_error_with_exit_code(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            harness = _make_harness(Path(temp_dir), created_thread=None, runner_exit_code=17)

            with self.assertRaises(new_command.NewCommandRunnerExitedError) as raised:
                new_command.run_new_command(
                    cwd=None,
                    prompt="hello",
                    abort=False,
                    create_timeout=0.1,
                    deps=harness.deps,
                )

        self.assertEqual(raised.exception.exit_code, 17)
        self.assertIn("(exit=17)", str(raised.exception))

    def test_unconfirmed_delivery_raises_typed_error(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            created_thread = _thread(root, "new-thread", rollout_exists=True)
            harness = _make_harness(root, created_thread=created_thread, delivery_thread=None)

            with self.assertRaises(new_command.NewCommandPromptDeliveryError) as raised:
                new_command.run_new_command(
                    cwd=None,
                    prompt="hello",
                    abort=False,
                    create_timeout=0.1,
                    deps=harness.deps,
                )

        self.assertEqual(str(raised.exception), "Prompt delivery could not be confirmed in the newly created thread.")
        self.assertIn("print:[new_thread_detected_by_list_diff] new-thread", harness.events)
        self.assertIn("selected:new-thread", harness.events)

    def test_missing_rollout_file_raises_unconfirmed_delivery_error(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            created_thread = _thread(root, "new-thread", rollout_exists=False)
            harness = _make_harness(root, created_thread=created_thread, delivery_thread=created_thread)

            with self.assertRaises(new_command.NewCommandPromptDeliveryError):
                new_command.run_new_command(
                    cwd=None,
                    prompt="hello",
                    abort=False,
                    create_timeout=0.1,
                    deps=harness.deps,
                )

        self.assertNotIn("delivery:hello:6.0", harness.events)
        self.assertNotIn("print:[background_runner_pid] 4321", harness.events)


def _make_harness(
    root: Path,
    *,
    created_thread: ThreadInfo | None,
    delivery_thread: ThreadInfo | None = None,
    runner_exit_code: int | None = None,
) -> NewCommandHarness:
    events: list[str] = []
    runner = FakeRunner(exit_code=runner_exit_code)

    def cancel_codex_reply_if_busy(timeout_sec: float) -> tuple[list[str], list[str]]:
        events.append(f"cancel:{timeout_sec}")
        return ["old"], []

    def resolve_new_thread_cwd(cwd: str | None) -> str:
        events.append(f"resolve:{cwd or '-'}")
        return str(root)

    def load_recent_threads(limit: int) -> list[ThreadInfo]:
        events.append(f"load:{limit}")
        return [_thread(root, "existing")]

    def spawn_background_new_thread_runner(prompt: str, cwd: str) -> FakeRunner:
        events.append(f"spawn:{prompt}:{cwd}")
        return runner

    def wait_for_new_thread(previous_ids: set[str], timeout_sec: float) -> ThreadInfo | None:
        events.append(f"wait:{','.join(sorted(previous_ids))}:{timeout_sec}")
        return created_thread

    def set_selected_thread_id(thread_id: str) -> None:
        events.append(f"selected:{thread_id}")

    def wait_for_prompt_delivery(
        session_offsets: dict[str, tuple[ThreadInfo, Path, int]],
        prompt: str,
        timeout_sec: float,
    ) -> ThreadInfo | None:
        _ = session_offsets
        events.append(f"delivery:{prompt}:{timeout_sec}")
        return delivery_thread

    def print_line(line: str) -> None:
        events.append(f"print:{line}")

    deps = new_command.NewCommandDeps(
        cancel_codex_reply_if_busy=cancel_codex_reply_if_busy,
        resolve_new_thread_cwd=resolve_new_thread_cwd,
        load_recent_threads=load_recent_threads,
        spawn_background_new_thread_runner=spawn_background_new_thread_runner,
        wait_for_new_thread=wait_for_new_thread,
        set_selected_thread_id=set_selected_thread_id,
        format_title_preview=lambda title: title,
        get_thread_ui_name=lambda _thread_id, _thread: "Thread UI",
        wait_for_prompt_delivery=wait_for_prompt_delivery,
        get_thread_label=lambda thread: thread.id,
        sync_session_index_with_state=lambda: 1,
        print_line=print_line,
    )
    return NewCommandHarness(deps=deps, events=events, runner=runner)


def _thread(root: Path, thread_id: str, *, rollout_exists: bool = False) -> ThreadInfo:
    rollout_path = root / f"{thread_id}.jsonl"
    if rollout_exists:
        _ = rollout_path.write_text("{}", encoding="utf-8")
    return ThreadInfo(
        id=thread_id,
        title=f"Title {thread_id}",
        cwd=str(root),
        updated_at=1,
        rollout_path=str(rollout_path),
        model="gpt",
        reasoning_effort="high",
        tokens_used=1,
    )


if __name__ == "__main__":
    _ = unittest.main()
