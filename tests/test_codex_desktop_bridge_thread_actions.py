from __future__ import annotations

import unittest

import codex_desktop_bridge_thread_actions as thread_actions
from codex_thread_models import ThreadInfo


class InterruptFailedError(RuntimeError):
    pass


class UnexpectedInterruptError(Exception):
    pass


def _thread(thread_id: str, title: str) -> ThreadInfo:
    return ThreadInfo(
        id=thread_id,
        title=title,
        cwd="C:/repo",
        updated_at=1,
        rollout_path="C:/repo/session.jsonl",
        model="gpt",
        reasoning_effort="high",
        tokens_used=1,
    )


def _deps(
    busy_sequences: list[list[ThreadInfo]],
    *,
    selected_thread_id: str | None = None,
    interrupt_error: Exception | None = None,
) -> tuple[thread_actions.ThreadActionDeps, list[str]]:
    busy_reads = list(busy_sequences)
    interrupted: list[str] = []
    now = [0.0]

    def get_busy_threads(limit: int) -> list[ThreadInfo]:
        _ = limit
        if busy_reads:
            return busy_reads.pop(0)
        return []

    def interrupt_thread_via_sidecar(thread: ThreadInfo) -> bool:
        interrupted.append(thread.id)
        if interrupt_error is not None:
            raise interrupt_error
        return True

    deps = thread_actions.ThreadActionDeps(
        load_recent_threads=lambda limit: [],
        get_busy_threads=get_busy_threads,
        get_thread_label=lambda thread: f"{thread.id}:{thread.title}",
        get_selected_thread_id=lambda: selected_thread_id,
        interrupt_thread_via_sidecar=interrupt_thread_via_sidecar,
        time_now=lambda: now[0],
        sleep=lambda seconds: now.__setitem__(0, now[0] + seconds),
    )
    return deps, interrupted


class DesktopBridgeThreadActionsTests(unittest.TestCase):
    def test_cancel_busy_reply_preserves_expected_interrupt_failure_fallback(self) -> None:
        busy = [_thread("thread-1", "First")]
        deps, interrupted = _deps([busy], interrupt_error=InterruptFailedError("sidecar unavailable"))

        cancelled, remaining = thread_actions.cancel_codex_reply_if_busy(1.0, deps)

        self.assertEqual(cancelled, ["thread-1:First"])
        self.assertEqual(remaining, ["thread-1:First"])
        self.assertEqual(interrupted, ["thread-1"])

    def test_cancel_busy_reply_surfaces_unexpected_interrupt_failure(self) -> None:
        busy = [_thread("thread-1", "First")]
        deps, interrupted = _deps([busy], interrupt_error=UnexpectedInterruptError("dependency broke"))

        with self.assertRaisesRegex(UnexpectedInterruptError, "dependency broke"):
            _ = thread_actions.cancel_codex_reply_if_busy(1.0, deps)

        self.assertEqual(interrupted, ["thread-1"])


if __name__ == "__main__":
    _ = unittest.main()
