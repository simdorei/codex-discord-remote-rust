from __future__ import annotations

from collections.abc import Mapping
from typing import cast
import unittest

import codex_discord_session_mirror_item_collection as item_collection
from codex_discord_session_mirror_detail import SessionMirrorDetailMode
from codex_discord_session_mirror_item_append import SessionPayload
from codex_session_events import JsonEvent, JsonValue


def _extract_message_text(payload: Mapping[str, JsonValue]) -> str:
    content = payload.get("content")
    if not isinstance(content, list):
        return str(payload.get("message") or "").strip()
    texts: list[str] = []
    for part in content:
        if isinstance(part, Mapping):
            part_mapping = cast(Mapping[str, JsonValue], part)
            texts.append(str(part_mapping.get("text") or ""))
    return "".join(texts).strip()


def _collect_items(
    events: list[JsonEvent],
    *,
    skip_texts: set[str] | None = None,
    detail_mode: SessionMirrorDetailMode = SessionMirrorDetailMode.SEND,
) -> list[dict[str, str]]:
    def should_skip_discord_origin_prompt(
        _codex_thread_id: str | None, text: str
    ) -> bool:
        return text in (skip_texts or set())

    def build_interactive_notice(payload: SessionPayload) -> str | None:
        _ = payload
        return None

    return item_collection.collect_session_mirror_items(
        "thread-1",
        events,
        seen_agent_messages={},
        seen_user_messages={},
        should_skip_discord_origin_prompt_func=should_skip_discord_origin_prompt,
        build_interactive_notice_func=build_interactive_notice,
        extract_message_text_func=_extract_message_text,
        recent_text_ttl_seconds=600.0,
        detail_mode=detail_mode,
    )


class SessionMirrorItemCollectionTests(unittest.TestCase):
    def test_collect_session_mirror_items_preserves_commentary_user_and_final_flow(
        self,
    ) -> None:
        events: list[JsonEvent] = [
            {
                "timestamp": "1",
                "type": "event_msg",
                "payload": {
                    "type": "agent_message",
                    "phase": "commentary",
                    "message": "working",
                },
            },
            {
                "timestamp": "2",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "user",
                    "content": [{"type": "input_text", "text": "from app"}],
                },
            },
            {
                "timestamp": "3",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "assistant",
                    "phase": "final_answer",
                    "content": [{"type": "output_text", "text": "done"}],
                },
            },
            {
                "timestamp": "4",
                "type": "event_msg",
                "payload": {
                    "type": "task_complete",
                    "turn_id": "turn-1",
                    "last_agent_message": "done",
                },
            },
        ]

        items = _collect_items(events)

        self.assertEqual(
            [item["kind"] for item in items], ["commentary", "user", "final"]
        )
        self.assertEqual(
            [item["text"] for item in items], ["working", "from app", "done"]
        )

    def test_collect_session_mirror_items_preserves_edge_skips_through_public_surface(
        self,
    ) -> None:
        events: list[JsonEvent] = [
            {"timestamp": "1", "type": "event_msg", "payload": "not-a-dict"},
            {
                "timestamp": "2",
                "type": "event_msg",
                "payload": {
                    "type": "agent_message",
                    "phase": "commentary",
                    "message": "   ",
                },
            },
            {
                "timestamp": "3",
                "type": "event_msg",
                "payload": {"type": "user_message", "message": "from discord"},
            },
            {
                "timestamp": "4",
                "type": "event_msg",
                "payload": {
                    "type": "agent_message",
                    "phase": "commentary",
                    "message": "repeat",
                },
            },
            {
                "timestamp": "5",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "assistant",
                    "phase": "commentary",
                    "content": [{"type": "output_text", "text": "repeat"}],
                },
            },
            {
                "timestamp": "6",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "user",
                    "content": [
                        {
                            "type": "input_text",
                            "text": "# AGENTS.md instructions for C:\\repos\\simdorei\\codex-discord-remote\n\n<INSTRUCTIONS>",
                        }
                    ],
                },
            },
            {
                "timestamp": "7",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "user",
                    "content": [{"type": "input_text", "text": "kept"}],
                },
            },
            {"timestamp": "8", "type": "unsupported", "payload": {"type": "message"}},
        ]

        items = _collect_items(events, skip_texts={"from discord"})

        self.assertEqual([item["kind"] for item in items], ["commentary", "user"])
        self.assertEqual([item["text"] for item in items], ["repeat", "kept"])

    def test_collect_session_mirror_items_preserves_aborted_and_rejected_items(
        self,
    ) -> None:
        events: list[JsonEvent] = [
            {
                "timestamp": "1",
                "type": "event_msg",
                "payload": {
                    "type": "task_aborted",
                    "task_id": "task-1",
                    "reason": "operator",
                },
            },
            {
                "timestamp": "2",
                "type": "event_msg",
                "payload": {"type": "task_cancelled"},
            },
            {
                "timestamp": "3",
                "type": "response_item",
                "payload": {
                    "type": "function_call_output",
                    "output": "Command rejected by user",
                },
            },
        ]

        items = _collect_items(events, detail_mode=SessionMirrorDetailMode.ALL)

        self.assertEqual(
            [item["kind"] for item in items],
            ["aborted", "aborted", "commentary", "commentary"],
        )
        self.assertEqual(items[0]["phase"], "task_aborted")
        self.assertIn("Codex task aborted.", items[0]["text"])
        self.assertIn("task_id=task-1", items[0]["text"])
        self.assertEqual(items[1]["text"], "Codex task cancelled.")
        self.assertEqual(items[2]["phase"], "tool_output")
        self.assertEqual(items[2]["text"], "Tool output:\nCommand rejected by user")
        self.assertEqual(items[3]["phase"], "approval_rejected")
        self.assertIn("[approval_rejected]", items[3]["text"])

    def test_collect_session_mirror_items_preserves_tool_image_outputs(self) -> None:
        events: list[JsonEvent] = [
            {
                "timestamp": "1",
                "type": "response_item",
                "payload": {
                    "type": "function_call_output",
                    "output": [
                        {
                            "type": "input_image",
                            "image_url": "data:image/png;base64,aGVsbG8=",
                        }
                    ],
                },
            }
        ]

        items = _collect_items(events)

        self.assertEqual(len(items), 1)
        self.assertEqual(items[0]["kind"], "image")
        self.assertEqual(items[0]["role"], "assistant")
        self.assertEqual(items[0]["phase"], "tool_image")
        self.assertEqual(items[0]["attachment_url"], "data:image/png;base64,aGVsbG8=")
        self.assertEqual(
            items[0]["attachment_filename"],
            "codex-image-output.png",
        )

if __name__ == "__main__":
    _ = unittest.main()
