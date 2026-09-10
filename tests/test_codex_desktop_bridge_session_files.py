# pyright: reportAny=false, reportPrivateUsage=false, reportUnknownArgumentType=false, reportUnknownMemberType=false, reportUnknownVariableType=false, reportUnusedCallResult=false
from __future__ import annotations

import json
import os
import tempfile
import unittest
from pathlib import Path
from typing import TypeAlias

import codex_desktop_bridge_session_files as session_files
from codex_bridge_state import JsonObject
from codex_thread_models import ThreadInfo

RolloutLine: TypeAlias = JsonObject | str


class SessionFileParserHappyTests(unittest.TestCase):
    def test_thread_from_rollout_path_uses_session_name_and_nested_settings(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            thread_id = "123e4567-e89b-12d3-a456-426614174000"
            path = root / f"rollout-{thread_id}.jsonl"
            _write_rollout(
                path,
                "",
                "{bad json",
                {"payload": {"type": "ignored"}},
                {
                    "type": "session_meta",
                    "payload": {"source": "vscode", "thread_source": "user", "cwd": "C:\\repo"},
                },
                {
                    "type": "turn_context",
                    "payload": {
                        "cwd": "C:\\repo2",
                        "model": "base-model",
                        "reasoning_effort": "low",
                        "collaboration_mode": {
                            "settings": {"model": "gpt-5.5", "reasoning_effort": "high"},
                        },
                    },
                },
                {"payload": {"type": "user_message", "message": "Fallback title\nignored"}},
            )
            os.utime(path, (1234.0, 1234.0))

            thread = _require_thread(
                session_files._thread_from_rollout_path(
                    path,
                    thread_id,
                    session_thread_names={thread_id: " Saved Title\nSecond line "},
                )
            )

            self.assertEqual(thread.id, thread_id)
            self.assertEqual(thread.title, "Saved Title")
            self.assertEqual(thread.cwd, "C:\\repo2")
            self.assertEqual(thread.updated_at, 1234)
            self.assertEqual(thread.rollout_path, str(path))
            self.assertEqual(thread.model, "gpt-5.5")
            self.assertEqual(thread.reasoning_effort, "high")
            self.assertEqual(thread.tokens_used, 0)


class SessionFileParserEdgeTests(unittest.TestCase):
    def test_rollout_parse_state_rejects_dynamic_attributes(self) -> None:
        state = session_files._RolloutParseState(title="Saved")

        with self.assertRaises(AttributeError):
            setattr(state, "extra", "not allowed")

        state.cwd = "C:\\repo"
        self.assertEqual((state.title, state.cwd), ("Saved", "C:\\repo"))

    def test_load_missing_rollout_threads_filters_and_uses_user_message_title(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            valid_id = "223e4567-e89b-12d3-a456-426614174000"
            existing_id = "323e4567-e89b-12d3-a456-426614174000"
            non_vscode_id = "423e4567-e89b-12d3-a456-426614174000"
            assistant_id = "523e4567-e89b-12d3-a456-426614174000"
            missing_cwd_id = "623e4567-e89b-12d3-a456-426614174000"
            missing_title_id = "723e4567-e89b-12d3-a456-426614174000"

            _write_valid_rollout(root / f"rollout-{valid_id}.jsonl", "User title\nsecond")
            _write_valid_rollout(root / f"rollout-{existing_id}.jsonl", "Existing title")
            _write_rollout(
                root / f"rollout-{non_vscode_id}.jsonl",
                _session_meta("cli", "user", "C:\\repo"),
                _turn_context("gpt-5", "medium"),
                _user_message("Wrong source"),
            )
            _write_rollout(
                root / f"rollout-{assistant_id}.jsonl",
                _session_meta("vscode", "assistant", "C:\\repo"),
                _turn_context("gpt-5", "medium"),
                _user_message("Assistant sourced"),
            )
            _write_rollout(
                root / f"rollout-{missing_cwd_id}.jsonl",
                {"type": "session_meta", "payload": {"source": "vscode", "thread_source": "user"}},
                _turn_context("gpt-5", "medium"),
                _user_message("Missing cwd"),
            )
            _write_rollout(
                root / f"rollout-{missing_title_id}.jsonl",
                _session_meta("vscode", "user", "C:\\repo"),
                _turn_context("gpt-5", "medium"),
            )
            _write_valid_rollout(root / "rollout-not-a-uuid.jsonl", "Invalid filename")

            threads = session_files.load_missing_vscode_rollout_threads(root, {existing_id})

            self.assertEqual([thread.id for thread in threads], [valid_id])
            thread = threads[0]
            self.assertEqual(thread.title, "User title")
            self.assertEqual(thread.cwd, "C:\\repo")
            self.assertEqual(thread.model, "gpt-5")
            self.assertEqual(thread.reasoning_effort, "medium")

    def test_missing_sessions_dir_returns_empty(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            missing = Path(temp_dir) / "missing"

            self.assertEqual(session_files.load_missing_vscode_rollout_threads(missing, set()), [])


def _write_valid_rollout(path: Path, user_message: str) -> None:
    _write_rollout(
        path,
        _session_meta("vscode", "user", "C:\\repo"),
        _user_message(user_message),
        _turn_context("gpt-5", "medium"),
    )


def _write_rollout(path: Path, *lines: RolloutLine) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    encoded_lines = [line if isinstance(line, str) else json.dumps(line) for line in lines]
    path.write_text("\n".join(encoded_lines) + "\n", encoding="utf-8")


def _session_meta(source: str, thread_source: str, cwd: str) -> JsonObject:
    return {
        "type": "session_meta",
        "payload": {"source": source, "thread_source": thread_source, "cwd": cwd},
    }


def _turn_context(model: str, reasoning_effort: str) -> JsonObject:
    return {
        "type": "turn_context",
        "payload": {"model": model, "reasoning_effort": reasoning_effort},
    }


def _user_message(message: str) -> JsonObject:
    return {"payload": {"type": "user_message", "message": message}}


def _require_thread(thread: ThreadInfo | None) -> ThreadInfo:
    if thread is None:
        raise AssertionError("Expected rollout file to produce a ThreadInfo.")
    return thread


if __name__ == "__main__":
    unittest.main()
