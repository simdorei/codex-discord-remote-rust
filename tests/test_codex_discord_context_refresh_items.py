from __future__ import annotations

import unittest

import codex_discord_context_refresh as context_refresh
from codex_session_events import JsonEvent, JsonValue


def _make_item(
    codex_thread_id: str,
    event: JsonEvent,
    *,
    kind: str,
    role: str,
    phase: str,
    text: str,
) -> dict[str, str]:
    _ = event
    clean_text = str(text or "").strip()
    return {
        "digest": f"{codex_thread_id}:{kind}:{role}:{phase}:{clean_text}",
        "kind": kind,
        "role": role,
        "phase": phase,
        "text": clean_text,
    }


def _interactive_notice(payload: dict[str, JsonValue]) -> str:
    return f"interactive:{payload.get('name') or 'unknown'}"


def _message_text(payload: dict[str, JsonValue]) -> str:
    return str(payload.get("text") or "").strip()


def _text_digest(*parts: str) -> str:
    return "|".join(parts)


class ContextRefreshItemTests(unittest.TestCase):
    def test_extract_context_refresh_item_reads_event_user_message(self) -> None:
        event: JsonEvent = {
            "type": "event_msg",
            "payload": {"type": "user_message", "message": "  hello from user  "},
        }

        item = context_refresh.extract_context_refresh_item(
            "thread-1",
            event,
            make_session_mirror_item_func=_make_item,
            build_interactive_notice_func=_interactive_notice,
            extract_message_text_func=_message_text,
        )

        self.assertEqual(
            item,
            {
                "digest": "thread-1:user:user:input:hello from user",
                "kind": "user",
                "role": "user",
                "phase": "input",
                "text": "hello from user",
            },
        )

    def test_extract_context_refresh_item_marks_event_agent_final_answer(self) -> None:
        event: JsonEvent = {
            "type": "event_msg",
            "payload": {
                "type": "agent_message",
                "phase": "final_answer",
                "message": "  complete  ",
            },
        }

        item = context_refresh.extract_context_refresh_item(
            "thread-1",
            event,
            make_session_mirror_item_func=_make_item,
            build_interactive_notice_func=_interactive_notice,
            extract_message_text_func=_message_text,
        )

        self.assertEqual(item["kind"] if item is not None else None, "final")
        self.assertEqual(item["role"] if item is not None else None, "assistant")
        self.assertEqual(item["phase"] if item is not None else None, "final_answer")
        self.assertEqual(item["text"] if item is not None else None, "complete")

    def test_extract_context_refresh_item_reads_function_call_notice(self) -> None:
        event: JsonEvent = {
            "type": "response_item",
            "payload": {"type": "function_call", "name": "shell"},
        }

        item = context_refresh.extract_context_refresh_item(
            "thread-1",
            event,
            make_session_mirror_item_func=_make_item,
            build_interactive_notice_func=_interactive_notice,
            extract_message_text_func=_message_text,
        )

        self.assertEqual(item["kind"] if item is not None else None, "interactive")
        self.assertEqual(item["role"] if item is not None else None, "assistant")
        self.assertEqual(item["phase"] if item is not None else None, "interactive")
        self.assertEqual(item["text"] if item is not None else None, "interactive:shell")

    def test_extract_context_refresh_item_ignores_malformed_or_empty_payloads(self) -> None:
        malformed_event: JsonEvent = {"type": "event_msg", "payload": "bad"}
        empty_message_event: JsonEvent = {
            "type": "response_item",
            "payload": {"type": "message", "role": "assistant", "text": "  "},
        }

        for event in [malformed_event, empty_message_event]:
            with self.subTest(event=event):
                self.assertIsNone(
                    context_refresh.extract_context_refresh_item(
                        "thread-1",
                        event,
                        make_session_mirror_item_func=_make_item,
                        build_interactive_notice_func=_interactive_notice,
                        extract_message_text_func=_message_text,
                    )
                )

    def test_collect_context_refresh_items_deduplicates_and_preserves_order(self) -> None:
        events: list[JsonEvent] = [
            {"type": "event_msg", "payload": {"type": "user_message", "message": " hello "}},
            {"type": "event_msg", "payload": {"type": "user_message", "message": "hello"}},
            {
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "assistant",
                    "phase": "final_answer",
                    "text": " done ",
                },
            },
        ]

        items = context_refresh.collect_context_refresh_items(
            "thread-1",
            events,
            make_session_mirror_item_func=_make_item,
            build_interactive_notice_func=_interactive_notice,
            extract_message_text_func=_message_text,
            make_text_digest_func=_text_digest,
        )

        self.assertEqual([item["text"] for item in items], ["hello", "done"])
        self.assertEqual([item["kind"] for item in items], ["user", "final"])

    def test_format_context_refresh_item_labels_and_truncates(self) -> None:
        self.assertEqual(
            context_refresh.format_context_refresh_item(
                {"kind": "final", "role": "assistant", "phase": "final_answer", "text": "done"},
                max_chars=200,
            ),
            "[assistant final]\ndone",
        )

        formatted = context_refresh.format_context_refresh_item(
            {
                "kind": "interactive",
                "role": "assistant",
                "phase": "interactive",
                "text": "x" * 140,
            },
            max_chars=105,
        )

        self.assertTrue(formatted.startswith("[assistant interactive]\n"))
        self.assertTrue(formatted.endswith("[truncated]"))
