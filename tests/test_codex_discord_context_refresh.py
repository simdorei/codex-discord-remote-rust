from __future__ import annotations

from datetime import datetime, timedelta, timezone
from pathlib import Path
import unittest

import codex_discord_context_refresh as context_refresh
import codex_discord_recent_user_prompt as recent_user_prompt
from codex_session_events import JsonEvent, JsonValue
from codex_thread_models import ThreadInfo


def _extract_message_text(_payload: dict[str, JsonValue]) -> str:
    return " extracted text "


class _FailingBridge:
    def __init__(self, exc: Exception) -> None:
        self.exc: Exception = exc
        self.seen_thread: tuple[str | None, str | None] | None = None

    def choose_thread(self, thread_id: str | None, cwd: str | None) -> ThreadInfo:
        self.seen_thread = (thread_id, cwd)
        raise self.exc

    def get_thread_workspace_ref(
        self,
        thread: ThreadInfo,
        threads: list[ThreadInfo] | None = None,
    ) -> str:
        _ = (thread, threads)
        raise AssertionError("workspace ref should not be requested")

    def get_thread_ui_name(self, thread_id: str, thread: ThreadInfo | None = None) -> str | None:
        _ = (thread_id, thread)
        raise AssertionError("thread name should not be requested")


def _unexpected_recent_events(_session_path: Path) -> list[JsonEvent]:
    raise AssertionError("session tail should not be read")


def _build_context_refresh_message_with_bridge(
    bridge: context_refresh.ContextRefreshBridge,
) -> str:
    return context_refresh.build_context_refresh_message(
        123,
        limit=3,
        max_chars=1000,
        bridge_module=bridge,
        get_mirrored_codex_thread_id_func=lambda _channel_id: "thread-1",
        resolve_selected_target_func=lambda: (None, ""),
        iter_recent_session_tail_events_func=_unexpected_recent_events,
        collect_context_refresh_items_func=lambda _thread_id, _events: [],
        format_context_refresh_item_func=lambda _item: "",
    )


class ContextRefreshTests(unittest.TestCase):
    def test_extract_user_text_from_session_event_reads_event_msg_user_message(self) -> None:
        event: dict[str, JsonValue] = {
            "type": "event_msg",
            "payload": {"type": "user_message", "message": " hello "},
        }

        self.assertEqual(
            context_refresh.extract_user_text_from_session_event(
                event,
                extract_message_text_func=_extract_message_text,
            ),
            "hello",
        )

    def test_extract_user_text_from_session_event_reads_response_item_user_message(self) -> None:
        event: dict[str, JsonValue] = {
            "type": "response_item",
            "payload": {"type": "message", "role": "user"},
        }

        self.assertEqual(
            context_refresh.extract_user_text_from_session_event(
                event,
                extract_message_text_func=_extract_message_text,
            ),
            "extracted text",
        )

    def test_extract_user_text_from_session_event_ignores_non_user_events(self) -> None:
        event: dict[str, JsonValue] = {
            "type": "response_item",
            "payload": {"type": "message", "role": "assistant"},
        }

        self.assertEqual(
            context_refresh.extract_user_text_from_session_event(
                event,
                extract_message_text_func=_extract_message_text,
            ),
            "",
        )

    def test_has_recent_user_prompt_matches_normalized_recent_prompt(self) -> None:
        now = datetime(2026, 6, 17, 2, 0, tzinfo=timezone.utc)
        events: list[dict[str, JsonValue]] = [
            {
                "timestamp": (now - timedelta(seconds=5)).isoformat(),
                "type": "event_msg",
                "payload": {"type": "user_message", "message": "  hello\nworld  "},
            },
        ]

        self.assertTrue(
            recent_user_prompt.has_recent_user_prompt(
                events,
                "hello world",
                max_age_seconds=30.0,
                now=now,
                normalize_prompt_text_func=lambda text: " ".join(text.split()),
                extract_user_text_func=lambda event: context_refresh.extract_user_text_from_session_event(
                    event,
                    extract_message_text_func=_extract_message_text,
                ),
                parse_timestamp_func=context_refresh.parse_session_event_timestamp,
            )
        )

    def test_build_context_refresh_message_reports_unavailable_thread_selection(self) -> None:
        bridge = _FailingBridge(RuntimeError("Thread not found: thread-1"))

        message = _build_context_refresh_message_with_bridge(bridge)

        self.assertEqual(
            message,
            "Context refresh unavailable.\n\nERROR: Thread not found: thread-1",
        )
        self.assertEqual(bridge.seen_thread, ("thread-1", None))

    def test_build_context_refresh_message_does_not_hide_unexpected_thread_selection_errors(
        self,
    ) -> None:
        bridge = _FailingBridge(ValueError("bad bridge state"))

        with self.assertRaisesRegex(ValueError, "bad bridge state"):
            _ = _build_context_refresh_message_with_bridge(bridge)
