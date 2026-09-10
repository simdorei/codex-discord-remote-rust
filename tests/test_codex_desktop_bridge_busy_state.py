# pyright: reportAny=false, reportAttributeAccessIssue=false, reportPrivateUsage=false, reportUnknownArgumentType=false, reportUnknownLambdaType=false, reportUnknownMemberType=false, reportUnknownVariableType=false, reportUnusedCallResult=false
from __future__ import annotations

from collections.abc import Iterator
import json
import os
import tempfile
import unittest
from pathlib import Path

import codex_desktop_bridge as bridge
import codex_desktop_bridge_busy_state as busy_state
from codex_session_events import JsonEvent, iter_session_events
from codex_thread_models import ThreadInfo


class BusyStateHappyTests(unittest.TestCase):
    def test_busy_state_sidecar_filtering_and_bridge_wrappers(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            busy_path = _session(root, "busy.jsonl", _task_started(), _user_message())
            idle_path = _session(root, "idle.jsonl", _task_started(), _task_complete())
            approval_path = _session(root, "approval.jsonl", _task_started(), _user_message())
            _touch(busy_path, 995.0)
            _touch(idle_path, 995.0)
            _touch(approval_path, 995.0)

            busy_thread = _thread(busy_path, "busy")
            idle_thread = _thread(idle_path, "idle")
            approval_thread = _thread(approval_path, "approval")
            sidecar = FakeSidecar(
                {
                    "busy": {"thread": {"status": {"type": "active", "activeFlags": []}}},
                    "idle": {"thread": {"status": {"type": "idle"}}},
                    "approval": {"thread": {"status": {"type": "notLoaded"}}},
                },
                loaded={"approval": {"status": {"type": "active", "activeFlags": ["waitingOnApproval"]}}},
            )
            deps = _deps(
                now=1000.0,
                threads=[busy_thread, idle_thread, approval_thread],
                sidecar=sidecar,
            )

            self.assertTrue(busy_state.is_thread_busy(busy_path, deps=deps))
            self.assertFalse(busy_state.is_thread_busy(idle_path, deps=deps))
            self.assertEqual(busy_state.get_thread_busy_state(busy_thread, client=sidecar, deps=deps), "busy")
            self.assertEqual(busy_state.get_thread_busy_state(idle_thread, client=sidecar, deps=deps), "idle")
            self.assertEqual(
                busy_state.get_thread_busy_state(approval_thread, client=sidecar, allow_resume=True, deps=deps),
                "waiting-approval",
            )
            self.assertEqual([thread.id for thread in busy_state.get_busy_threads(deps=deps)], ["busy", "approval"])

            original_deps = bridge._make_busy_state_deps
            try:
                bridge._make_busy_state_deps = lambda: deps
                self.assertTrue(bridge.is_thread_busy(busy_path))
                self.assertEqual(bridge.get_thread_busy_state(approval_thread, allow_resume=True), "waiting-approval")
                self.assertIn("approval prompt", bridge.describe_thread_busy_state("waiting-approval"))
            finally:
                bridge._make_busy_state_deps = original_deps

            self.assertTrue(sidecar.closed)


class BusyStateEdgeTests(unittest.TestCase):
    def test_missing_malformed_stale_orphan_and_sidecar_edges(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            missing = root / "missing.jsonl"
            malformed = _session(root, "malformed.jsonl", {"type": "event_msg", "payload": []}, {"payload": {"type": "noop"}})
            orphan = _session(root, "orphan.jsonl", _task_started())
            stale = _session(root, "stale.jsonl", _task_started(), _user_message(), _agent_message())
            interactive = _session(root, "interactive.jsonl", _task_started())
            load_failure = _session(root, "load_failure.jsonl", _task_started(), _user_message())
            _touch(orphan, 900.0)
            _touch(stale, 900.0)
            _touch(interactive, 900.0)
            _touch(load_failure, 995.0)

            deps = _deps(
                now=1000.0,
                orphan_grace=60.0,
                stale_seconds=60.0,
                interactive={interactive: "waiting-input"},
                sidecar=FailingSidecar(),
            )

            self.assertIsNone(busy_state.session_file_age_seconds(missing, now=1000.0))
            self.assertFalse(busy_state.is_thread_busy(missing, deps=deps))
            self.assertFalse(busy_state.is_thread_busy(malformed, deps=deps))
            self.assertFalse(busy_state.is_thread_busy(orphan, deps=deps))
            self.assertFalse(busy_state.is_thread_busy(stale, deps=deps))
            self.assertTrue(busy_state.is_thread_busy(interactive, deps=deps))
            self.assertEqual(busy_state.get_thread_busy_state(_thread(interactive, "interactive"), client=FailingSidecar(), deps=deps), "waiting-input")
            self.assertEqual(busy_state.get_thread_busy_state(_thread(stale, "stale"), client=FailingSidecar(), deps=deps), "idle")
            self.assertEqual(
                busy_state.get_thread_busy_state(
                    _thread(load_failure, "load_failure"),
                    client=FakeSidecar(
                        {"load_failure": {"thread": {"status": {"type": "notLoaded"}}}},
                    ),
                    allow_resume=True,
                    deps=deps,
                ),
                "busy",
            )
            self.assertIsNone(busy_state.classify_thread_status({}))
            self.assertEqual(busy_state.classify_thread_status({"type": "saving"}), "busy")
            self.assertEqual(
                busy_state.classify_thread_status({"type": "active", "activeFlags": ["waitingOnUserInput"]}),
                "waiting-input",
            )

    def test_is_thread_busy_scans_events_without_materializing_iterator(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            session_path = Path(temp_dir) / "streaming.jsonl"
            session_path.write_text("", encoding="utf-8")

            def iter_events(path: Path) -> Iterator[JsonEvent]:
                self.assertEqual(path, session_path)
                return NoLengthHintEvents([_task_started(), _user_message()])

            deps = _deps()
            streaming_deps = busy_state.BusyStateDeps(
                iter_session_events=iter_events,
                time_now=deps.time_now,
                get_orphan_task_started_grace_seconds=deps.get_orphan_task_started_grace_seconds,
                get_stale_busy_session_seconds=deps.get_stale_busy_session_seconds,
                get_pending_interactive_state_from_session=deps.get_pending_interactive_state_from_session,
                load_recent_threads=deps.load_recent_threads,
                make_sidecar=deps.make_sidecar,
                get_sidecar_thread_status_type=deps.get_sidecar_thread_status_type,
                ensure_thread_loaded_via_sidecar=deps.ensure_thread_loaded_via_sidecar,
            )

            self.assertTrue(busy_state.is_thread_busy(session_path, deps=streaming_deps))


class FakeSidecar:
    def __init__(self, responses: dict[str, busy_state.JsonObject], *, loaded: dict[str, busy_state.JsonObject] | None = None) -> None:
        self.responses: dict[str, busy_state.JsonObject] = responses
        self.loaded: dict[str, busy_state.JsonObject] = loaded or {}
        self.closed: bool = False

    def read_thread(self, thread_id: str, *, include_turns: bool = False) -> busy_state.JsonObject:
        _ = include_turns
        return self.responses[thread_id]

    def ensure_loaded(self, thread_id: str) -> busy_state.JsonObject:
        return self.loaded[thread_id]

    def close(self) -> None:
        self.closed = True


class FailingSidecar:
    def read_thread(self, thread_id: str, *, include_turns: bool = False) -> busy_state.JsonObject:
        _ = thread_id, include_turns
        raise RuntimeError("sidecar failed")

    def close(self) -> None:
        pass


class NoLengthHintEvents:
    def __init__(self, events: list[JsonEvent]) -> None:
        self._events = iter(events)

    def __iter__(self) -> Iterator[JsonEvent]:
        return self

    def __next__(self) -> JsonEvent:
        return next(self._events)

    def __length_hint__(self) -> int:
        raise AssertionError("session events were materialized")


def _deps(
    *,
    now: float = 1000.0,
    orphan_grace: float = 60.0,
    stale_seconds: float = 1800.0,
    interactive: dict[Path, str] | None = None,
    threads: list[ThreadInfo] | None = None,
    sidecar: FakeSidecar | FailingSidecar | None = None,
) -> busy_state.BusyStateDeps:
    interactive_map = interactive or {}
    sidecar_client = sidecar or FakeSidecar({})
    return busy_state.BusyStateDeps(
        iter_session_events=iter_session_events,
        time_now=lambda: now,
        get_orphan_task_started_grace_seconds=lambda: orphan_grace,
        get_stale_busy_session_seconds=lambda: stale_seconds,
        get_pending_interactive_state_from_session=lambda path: interactive_map.get(path),
        load_recent_threads=lambda limit: list(threads or []),
        make_sidecar=lambda: sidecar_client,
        get_sidecar_thread_status_type=_status_type,
        ensure_thread_loaded_via_sidecar=lambda client, thread_id: client.ensure_loaded(thread_id),
    )


def _status_type(thread_payload: busy_state.JsonObject) -> str:
    status = thread_payload.get("status")
    if not isinstance(status, dict):
        return ""
    return str(status.get("type") or "").strip()


def _thread(session_path: Path, thread_id: str) -> ThreadInfo:
    return ThreadInfo(
        id=thread_id,
        title=thread_id,
        cwd=str(session_path.parent),
        updated_at=1,
        rollout_path=str(session_path),
        model="gpt",
        reasoning_effort="high",
        tokens_used=1,
    )


def _session(root: Path, name: str, *events: busy_state.JsonObject) -> Path:
    path = root / name
    path.write_text("\n".join(json.dumps(event) for event in events) + "\n", encoding="utf-8")
    return path


def _touch(path: Path, timestamp: float) -> None:
    os.utime(path, (timestamp, timestamp))


def _task_started() -> busy_state.JsonObject:
    return {"type": "event_msg", "payload": {"type": "task_started"}}


def _task_complete() -> busy_state.JsonObject:
    return {"type": "event_msg", "payload": {"type": "task_complete"}}


def _user_message() -> busy_state.JsonObject:
    return {"type": "event_msg", "payload": {"type": "user_message", "message": "run"}}


def _agent_message() -> busy_state.JsonObject:
    return {"type": "event_msg", "payload": {"type": "agent_message", "phase": "commentary", "message": "working"}}


if __name__ == "__main__":
    unittest.main()
