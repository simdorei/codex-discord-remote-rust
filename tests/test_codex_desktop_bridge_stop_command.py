from __future__ import annotations

import unittest

import codex_desktop_bridge_stop_command as stop_command
from codex_thread_models import ThreadInfo


class ActiveTurnLookupFailure(Exception):
    pass


def _thread() -> ThreadInfo:
    return ThreadInfo(
        id="thread-1",
        title="Thread title",
        cwd="C:/repo",
        updated_at=1,
        rollout_path="C:/repo/session.jsonl",
        model="gpt-5.5",
        reasoning_effort="high",
        tokens_used=42,
    )


def _deps(
    *,
    lines: list[str],
    interrupt_calls: list[tuple[str, str]],
    active_turn_reads: list[str | None],
) -> stop_command.StopCommandDeps:
    active_sequences = list(active_turn_reads)
    now = [0.0]

    def get_active_turn_id(thread_id: str) -> str | None:
        _ = thread_id
        if active_sequences:
            return active_sequences.pop(0)
        return None

    def interrupt_turn(thread_id: str, turn_id: str) -> object:
        interrupt_calls.append((thread_id, turn_id))
        return {}

    return stop_command.StopCommandDeps(
        get_active_turn_id=get_active_turn_id,
        interrupt_turn=interrupt_turn,
        get_thread_label=lambda thread: f"{thread.title} ({thread.id})",
        time_now=lambda: now[0],
        sleep=lambda seconds: now.__setitem__(0, now[0] + seconds),
        print_line=lines.append,
    )


class DesktopBridgeStopCommandTests(unittest.TestCase):
    def test_run_stop_command_interrupts_target_thread_and_confirms_idle(self) -> None:
        interrupt_calls: list[tuple[str, str]] = []
        lines: list[str] = []

        stop_command.run_stop_command(
            _thread(),
            deps=_deps(
                lines=lines,
                interrupt_calls=interrupt_calls,
                active_turn_reads=["turn-1", "turn-1", None],
            ),
        )

        self.assertEqual(interrupt_calls, [("thread-1", "turn-1")])
        self.assertEqual(
            lines,
            [
                "target_thread: Thread title (thread-1)",
                "reply_stop_requested: true",
                "reply_stop_confirmed: true",
            ],
        )

    def test_run_stop_command_reports_pending_when_target_stays_busy(self) -> None:
        interrupt_calls: list[tuple[str, str]] = []
        lines: list[str] = []

        stop_command.run_stop_command(
            _thread(),
            deps=_deps(
                lines=lines,
                interrupt_calls=interrupt_calls,
                active_turn_reads=["turn-1" for _ in range(20)],
            ),
        )

        self.assertEqual(interrupt_calls, [("thread-1", "turn-1")])
        self.assertEqual(
            lines,
            [
                "target_thread: Thread title (thread-1)",
                "reply_stop_requested: true",
                "reply_stop_confirmed: false",
            ],
        )

    def test_run_stop_command_reports_not_requested_without_active_turn(self) -> None:
        interrupt_calls: list[tuple[str, str]] = []
        lines: list[str] = []

        stop_command.run_stop_command(
            _thread(),
            deps=_deps(
                lines=lines,
                interrupt_calls=interrupt_calls,
                active_turn_reads=[None],
            ),
        )

        self.assertEqual(interrupt_calls, [])
        self.assertEqual(
            lines,
            [
                "target_thread: Thread title (thread-1)",
                "reply_stop_requested: false",
                "reply_stop_confirmed: false",
            ],
        )

    def test_run_stop_command_surfaces_active_turn_lookup_failure(self) -> None:
        interrupt_calls: list[tuple[str, str]] = []
        lines: list[str] = []

        def get_active_turn_id(thread_id: str) -> str | None:
            _ = thread_id
            raise ActiveTurnLookupFailure("active turn read failed")

        with self.assertRaisesRegex(ActiveTurnLookupFailure, "active turn read failed"):
            stop_command.run_stop_command(
                _thread(),
                deps=stop_command.StopCommandDeps(
                    get_active_turn_id=get_active_turn_id,
                    interrupt_turn=lambda thread_id, turn_id: interrupt_calls.append((thread_id, turn_id)),
                    get_thread_label=lambda thread: f"{thread.title} ({thread.id})",
                    time_now=lambda: 0.0,
                    sleep=lambda seconds: None,
                    print_line=lines.append,
                ),
            )

        self.assertEqual(interrupt_calls, [])
        self.assertEqual(lines, ["target_thread: Thread title (thread-1)"])

    def test_run_stop_command_surfaces_confirm_lookup_failure(self) -> None:
        interrupt_calls: list[tuple[str, str]] = []
        lines: list[str] = []
        active_turn_reads: list[str | ActiveTurnLookupFailure] = [
            "turn-1",
            ActiveTurnLookupFailure("confirm read failed"),
        ]

        def get_active_turn_id(thread_id: str) -> str | None:
            _ = thread_id
            value = active_turn_reads.pop(0)
            if isinstance(value, ActiveTurnLookupFailure):
                raise value
            return value

        with self.assertRaisesRegex(ActiveTurnLookupFailure, "confirm read failed"):
            stop_command.run_stop_command(
                _thread(),
                deps=stop_command.StopCommandDeps(
                    get_active_turn_id=get_active_turn_id,
                    interrupt_turn=lambda thread_id, turn_id: interrupt_calls.append((thread_id, turn_id)),
                    get_thread_label=lambda thread: f"{thread.title} ({thread.id})",
                    time_now=lambda: 0.0,
                    sleep=lambda seconds: None,
                    print_line=lines.append,
                ),
            )

        self.assertEqual(interrupt_calls, [("thread-1", "turn-1")])
        self.assertEqual(
            lines,
            [
                "target_thread: Thread title (thread-1)",
                "reply_stop_requested: true",
            ],
        )

    def test_run_stop_command_surfaces_sidecar_failure(self) -> None:
        def interrupt_turn(thread_id: str, turn_id: str) -> object:
            _ = thread_id
            _ = turn_id
            raise RuntimeError("sidecar unavailable")

        with self.assertRaisesRegex(RuntimeError, "sidecar unavailable"):
            stop_command.run_stop_command(
                _thread(),
                deps=stop_command.StopCommandDeps(
                    get_active_turn_id=lambda thread_id: "turn-1",
                    interrupt_turn=interrupt_turn,
                    get_thread_label=lambda thread: thread.id,
                    time_now=lambda: 0.0,
                    sleep=lambda seconds: None,
                    print_line=lambda line: None,
                ),
            )


if __name__ == "__main__":
    _ = unittest.main()
