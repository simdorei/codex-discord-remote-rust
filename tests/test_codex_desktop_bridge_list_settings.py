from __future__ import annotations

import io
import json
import sqlite3
import tempfile
import unittest
from argparse import Namespace
from contextlib import redirect_stdout
from pathlib import Path

import codex_desktop_bridge as bridge
from codex_thread_models import ThreadInfo


class DesktopBridgeListSettingsTests(unittest.TestCase):
    def test_load_user_root_threads_ignores_rollout_missing_from_state_db(self) -> None:
        original_codex_home = bridge.CODEX_HOME
        original_state_db_path = bridge.STATE_DB_PATH
        try:
            with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
                codex_home = Path(temp_dir) / "codex-home"
                sessions_dir = codex_home / "sessions" / "2026" / "06" / "17"
                sessions_dir.mkdir(parents=True)
                bridge.CODEX_HOME = codex_home
                bridge.STATE_DB_PATH = Path(temp_dir) / "state.sqlite"
                missing_id = "11111111-1111-4111-8111-111111111111"
                missing_session = sessions_dir / (
                    f"rollout-2026-06-17T11-38-13-{missing_id}.jsonl"
                )
                _ = missing_session.write_text(
                    "\n".join(
                        json.dumps(event)
                        for event in [
                            {
                                "type": "session_meta",
                                "payload": {
                                    "cwd": str(Path(temp_dir) / "repo"),
                                    "source": "vscode",
                                },
                            },
                            {
                                "type": "event_msg",
                                "payload": {
                                    "type": "user_message",
                                    "message": "new local session\nsecond line",
                                },
                            },
                            {
                                "type": "turn_context",
                                "payload": {
                                    "model": "gpt-test",
                                    "reasoning_effort": "high",
                                },
                            },
                        ]
                    )
                    + "\n",
                    encoding="utf-8",
                )

                with sqlite3.connect(bridge.STATE_DB_PATH) as conn:
                    _ = conn.execute(
                        " ".join(
                            [
                                "CREATE TABLE threads (",
                                "id TEXT PRIMARY KEY,",
                                "title TEXT NOT NULL,",
                                "cwd TEXT NOT NULL,",
                                "updated_at INTEGER NOT NULL,",
                                "rollout_path TEXT NOT NULL,",
                                "model TEXT,",
                                "reasoning_effort TEXT,",
                                "tokens_used INTEGER NOT NULL DEFAULT 0,",
                                "archived INTEGER NOT NULL DEFAULT 0,",
                                "source TEXT NOT NULL,",
                                "thread_source TEXT",
                                ")",
                            ]
                        )
                    )
                    _ = conn.execute(
                        " ".join(
                            [
                                "INSERT INTO threads (",
                                "id, title, cwd, updated_at, rollout_path, model,",
                                "reasoning_effort, tokens_used, archived, source,",
                                "thread_source",
                                ") VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                            ]
                        ),
                        (
                            "root-1",
                            "root",
                            str(Path(temp_dir) / "repo"),
                            1,
                            str(Path(temp_dir) / "root.jsonl"),
                            "gpt-test",
                            "high",
                            10,
                            0,
                            "vscode",
                            None,
                        ),
                    )

                threads = bridge.load_user_root_threads()

            self.assertEqual([thread.id for thread in threads], ["root-1"])
        finally:
            bridge.CODEX_HOME = original_codex_home
            bridge.STATE_DB_PATH = original_state_db_path

    def test_command_list_keeps_db_root_list_global_when_target_thread_exists(self) -> None:
        original_load_ordinary_user_root_threads = bridge.load_ordinary_user_root_threads
        original_choose_thread = bridge.choose_thread
        original_print_thread_list = bridge.print_thread_list
        try:
            target_thread = ThreadInfo(
                id="target",
                title="Target",
                cwd="C:\\repo\\one",
                updated_at=3,
                rollout_path="",
                model="",
                reasoning_effort="",
                tokens_used=0,
            )
            same_project = ThreadInfo(
                id="same",
                title="Same",
                cwd="C:\\repo\\one",
                updated_at=2,
                rollout_path="",
                model="",
                reasoning_effort="",
                tokens_used=0,
            )
            other_project = ThreadInfo(
                id="other",
                title="Other",
                cwd="C:\\repo\\two",
                updated_at=1,
                rollout_path="",
                model="",
                reasoning_effort="",
                tokens_used=0,
            )
            printed: list[list[str]] = []
            observed_limits: list[int] = []

            def load_ordinary_user_root_threads(limit: int = 0) -> list[ThreadInfo]:
                observed_limits.append(limit)
                return [target_thread, same_project, other_project]

            bridge.load_ordinary_user_root_threads = load_ordinary_user_root_threads
            bridge.choose_thread = lambda thread_id, cwd: target_thread
            bridge.print_thread_list = lambda threads: printed.append([thread.id for thread in threads])

            result = bridge.command_list(
                Namespace(db_root=True, limit=0, thread_id="target", cwd=None)
            )

            self.assertEqual(result, 0)
            self.assertEqual(printed, [["target", "same", "other"]])
            self.assertEqual(observed_limits, [50])
        finally:
            bridge.load_ordinary_user_root_threads = original_load_ordinary_user_root_threads
            bridge.choose_thread = original_choose_thread
            bridge.print_thread_list = original_print_thread_list

    def test_print_thread_list_includes_model_reasoning_mode_and_speed(self) -> None:
        original_selected = bridge.get_selected_thread_id
        original_refs = bridge.build_workspace_ref_map
        original_ui_name = bridge.get_thread_ui_name
        original_workspace = bridge.get_thread_workspace_name
        original_busy = bridge.is_thread_busy
        original_context = bridge.get_thread_context_usage
        original_recommend = bridge.should_recommend_archive
        try:
            with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
                session_path = Path(temp_dir) / "session.jsonl"
                events = [
                    {
                        "type": "event_msg",
                        "payload": {"type": "task_started", "collaboration_mode_kind": "default"},
                    },
                    {
                        "type": "turn_context",
                        "payload": {
                            "collaboration_mode": {"mode": "default"},
                            "service_tier": "priority",
                        },
                    },
                ]
                _ = session_path.write_text(
                    "\n".join(json.dumps(event) for event in events) + "\n",
                    encoding="utf-8",
                )
                thread = ThreadInfo(
                    id="thread-1",
                    title="Thread",
                    cwd="C:\\repo",
                    updated_at=1,
                    rollout_path=str(session_path),
                    model="gpt-5.5",
                    reasoning_effort="xhigh",
                    tokens_used=1234,
                )
                second_thread = ThreadInfo(
                    id="thread-2",
                    title="Second",
                    cwd="C:\\repo",
                    updated_at=2,
                    rollout_path=str(session_path),
                    model="gpt-5.5",
                    reasoning_effort="xhigh",
                    tokens_used=5678,
                )
                bridge.get_selected_thread_id = lambda: ""
                bridge.build_workspace_ref_map = lambda threads: {"thread-1": "repo", "thread-2": "repo"}
                bridge.get_thread_ui_name = lambda thread_id, thread=None: "Second" if thread_id == "thread-2" else "Thread"
                bridge.get_thread_workspace_name = lambda item: "repo"
                bridge.is_thread_busy = lambda path: False
                bridge.get_thread_context_usage = lambda item: None
                bridge.should_recommend_archive = lambda thread, context_usage: False

                output = io.StringIO()
                with redirect_stdout(output):
                    bridge.print_thread_list([thread, second_thread])

                text = output.getvalue()
                self.assertIn("model gpt-5.5/xhigh/default/fast", text)
                lines = text.splitlines()
                self.assertEqual(lines[1], "")
                self.assertIn("Second", lines[2])
        finally:
            bridge.get_selected_thread_id = original_selected
            bridge.build_workspace_ref_map = original_refs
            bridge.get_thread_ui_name = original_ui_name
            bridge.get_thread_workspace_name = original_workspace
            bridge.is_thread_busy = original_busy
            bridge.get_thread_context_usage = original_context
            bridge.should_recommend_archive = original_recommend
