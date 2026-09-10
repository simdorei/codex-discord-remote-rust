from __future__ import annotations

import unittest

import codex_discord_session_mirror_activity_items as activity_items
import codex_discord_session_mirror_item_collection as item_collection
from codex_discord_session_mirror_detail import SessionMirrorDetailMode
from codex_discord_session_mirror_item_append import SessionPayload
from codex_session_events import JsonEvent


class SessionMirrorActivityItemTests(unittest.TestCase):
    def test_detail_mode_filters_visible_reasoning_and_raw_tool_activity(self) -> None:
        events: list[JsonEvent] = [
            {
                "timestamp": "1",
                "type": "event_msg",
                "payload": {
                    "type": "agent_message",
                    "phase": "commentary",
                    "message": "normal update",
                },
            },
            {
                "timestamp": "2",
                "type": "response_item",
                "payload": {
                    "type": "reasoning",
                    "summary": [{"type": "summary_text", "text": "visible reasoning"}],
                },
            },
            {
                "timestamp": "3",
                "type": "response_item",
                "payload": {
                    "type": "function_call",
                    "name": "exec_command",
                    "arguments": '{"cmd":"Get-Date"}',
                },
            },
        ]

        def should_skip_discord_origin_prompt(
            thread_id: str | None,
            text: str,
        ) -> bool:
            _ = (thread_id, text)
            return False

        def build_interactive_notice(payload: SessionPayload) -> str:
            _ = payload
            return ""

        def extract_message_text(payload: SessionPayload) -> str:
            _ = payload
            return ""

        def collect(detail_mode: SessionMirrorDetailMode) -> list[dict[str, str]]:
            return item_collection.collect_session_mirror_items(
                "thread-1",
                events,
                seen_agent_messages={},
                seen_user_messages={},
                should_skip_discord_origin_prompt_func=(
                    should_skip_discord_origin_prompt
                ),
                build_interactive_notice_func=build_interactive_notice,
                extract_message_text_func=extract_message_text,
                recent_text_ttl_seconds=60.0,
                detail_mode=detail_mode,
            )

        send_items = collect(SessionMirrorDetailMode.SEND)
        all_items = collect(SessionMirrorDetailMode.ALL)

        self.assertEqual(
            [(item["phase"], item["text"]) for item in send_items],
            [("commentary", "normal update")],
        )
        self.assertEqual(
            [(item["phase"], item["text"]) for item in all_items],
            [
                ("commentary", "normal update"),
                ("reasoning", "visible reasoning"),
                ("tool_call", 'Tool call: exec_command\nInput:\n{"cmd":"Get-Date"}'),
            ],
        )

    def test_collection_uses_regular_commentary_with_full_visible_event_text(
        self,
    ) -> None:
        events: list[JsonEvent] = [
            {
                "timestamp": "1",
                "type": "response_item",
                "payload": {
                    "type": "reasoning",
                    "summary": [
                        {"type": "summary_text", "text": "private chain of thought"}
                    ],
                },
            },
            {
                "timestamp": "2",
                "type": "response_item",
                "payload": {
                    "type": "function_call",
                    "name": "exec_command",
                    "arguments": '{"cmd":"print secret"}',
                    "call_id": "call-1",
                },
            },
            {
                "timestamp": "3",
                "type": "response_item",
                "payload": {
                    "type": "custom_tool_call",
                    "name": "apply_patch",
                    "input": "*** private patch contents",
                    "call_id": "call-2",
                },
            },
        ]

        def should_skip_discord_origin_prompt(thread_id: str | None, text: str) -> bool:
            _ = (thread_id, text)
            return False

        def build_interactive_notice(payload: SessionPayload) -> str:
            _ = payload
            return ""

        def extract_message_text(payload: SessionPayload) -> str:
            _ = payload
            return ""

        items = item_collection.collect_session_mirror_items(
            "thread-1",
            events,
            seen_agent_messages={},
            seen_user_messages={},
            should_skip_discord_origin_prompt_func=should_skip_discord_origin_prompt,
            build_interactive_notice_func=build_interactive_notice,
            extract_message_text_func=extract_message_text,
            recent_text_ttl_seconds=60.0,
            detail_mode=SessionMirrorDetailMode.ALL,
        )

        self.assertEqual(
            [item["kind"] for item in items],
            ["commentary", "commentary", "commentary"],
        )
        self.assertEqual(
            [(item["phase"], item["text"]) for item in items],
            [
                ("reasoning", "private chain of thought"),
                (
                    "tool_call",
                    'Tool call: exec_command\nInput:\n{"cmd":"print secret"}',
                ),
                (
                    "tool_call",
                    "Tool call: apply_patch\nInput:\n*** private patch contents",
                ),
            ],
        )

    def test_tool_output_preserves_all_text_parts_and_leaves_attachments_to_existing_path(
        self,
    ) -> None:
        payload: SessionPayload = {
            "type": "function_call_output",
            "output": [
                {"type": "input_text", "text": "first output\nwith spacing"},
                {"type": "input_image", "image_url": "data:image/png;base64,secret"},
                {"type": "input_text", "text": "second output"},
            ],
        }

        items = activity_items.build_activity_items(payload)

        self.assertEqual(
            items,
            (
                activity_items.ActivityItem(
                    phase="tool_output",
                    text="Tool output:\nfirst output\nwith spacing",
                ),
                activity_items.ActivityItem(
                    phase="tool_output",
                    text="Tool output:\nsecond output",
                ),
            ),
        )

    def test_reasoning_exposes_summary_but_not_encrypted_internal_content(self) -> None:
        payload: SessionPayload = {
            "type": "reasoning",
            "summary": [
                {"type": "summary_text", "text": "visible summary one"},
                {"type": "summary_text", "text": "visible summary two"},
            ],
            "encrypted_content": "hidden chain of thought",
        }

        items = activity_items.build_activity_items(payload)

        self.assertEqual(
            items,
            (
                activity_items.ActivityItem(
                    phase="reasoning",
                    text="visible summary one",
                ),
                activity_items.ActivityItem(
                    phase="reasoning",
                    text="visible summary two",
                ),
            ),
        )
        self.assertNotIn("hidden chain of thought", repr(items))

    def test_collection_preserves_whitespace_and_duplicate_activity_parts(
        self,
    ) -> None:
        events: list[JsonEvent] = [
            {
                "timestamp": "1",
                "type": "response_item",
                "payload": {
                    "type": "function_call_output",
                    "output": [
                        {"type": "input_text", "text": "  repeated output  \n"},
                        {"type": "input_text", "text": "  repeated output  \n"},
                    ],
                },
            }
        ]

        def should_skip_discord_origin_prompt(thread_id: str | None, text: str) -> bool:
            _ = (thread_id, text)
            return False

        def build_interactive_notice(payload: SessionPayload) -> str:
            _ = payload
            return ""

        def extract_message_text(payload: SessionPayload) -> str:
            _ = payload
            return ""

        items = item_collection.collect_session_mirror_items(
            "thread-1",
            events,
            seen_agent_messages={},
            seen_user_messages={},
            should_skip_discord_origin_prompt_func=should_skip_discord_origin_prompt,
            build_interactive_notice_func=build_interactive_notice,
            extract_message_text_func=extract_message_text,
            recent_text_ttl_seconds=60.0,
            detail_mode=SessionMirrorDetailMode.ALL,
        )

        self.assertEqual(len(items), 2)
        self.assertEqual(
            [item["text"] for item in items],
            [
                "Tool output:\n  repeated output  \n",
                "Tool output:\n  repeated output  \n",
            ],
        )
        self.assertNotEqual(items[0]["digest"], items[1]["digest"])


if __name__ == "__main__":
    _ = unittest.main()
