from __future__ import annotations

import unittest
from pathlib import Path
from tempfile import TemporaryDirectory
from unittest import mock

from codex_session_events import JsonEvent
import codex_discord_delivery_runtime as delivery_runtime
import codex_discord_session_mirror_item_collection as item_collection


def _collect_items(events: list[JsonEvent]) -> list[dict[str, str]]:
    return item_collection.collect_session_mirror_items(
        "thread-1",
        events,
        seen_agent_messages={},
        seen_user_messages={},
        should_skip_discord_origin_prompt_func=lambda _thread_id, _text: False,
        build_interactive_notice_func=lambda _payload: None,
        extract_message_text_func=lambda _payload: "",
        recent_text_ttl_seconds=600.0,
    )


class SessionMirrorFileAttachmentTests(unittest.TestCase):
    def test_collect_session_mirror_items_preserves_multiple_tool_images(self) -> None:
        events: list[JsonEvent] = [
            {
                "timestamp": "1",
                "type": "response_item",
                "payload": {
                    "type": "function_call_output",
                    "output": [
                        {"type": "input_image", "image_url": "data:image/png;base64,aGVsbG8="},
                        {"type": "input_image", "image_url": "data:image/png;base64,d29ybGQ="},
                    ],
                },
            }
        ]

        items = _collect_items(events)

        self.assertEqual([item["kind"] for item in items], ["image", "image"])
        self.assertEqual(
            [item["attachment_url"] for item in items],
            ["data:image/png;base64,aGVsbG8=", "data:image/png;base64,d29ybGQ="],
        )
        self.assertNotEqual(items[0]["digest"], items[1]["digest"])

    def test_collect_session_mirror_items_preserves_custom_tool_images(self) -> None:
        events: list[JsonEvent] = [
            {
                "timestamp": "1",
                "type": "response_item",
                "payload": {
                    "type": "custom_tool_call_output",
                    "output": [
                        {"type": "input_image", "image_url": "data:image/png;base64,aGVsbG8="}
                    ],
                },
            }
        ]

        items = _collect_items(events)

        self.assertEqual(len(items), 1)
        self.assertEqual(items[0]["kind"], "image")
        self.assertEqual(items[0]["attachment_url"], "data:image/png;base64,aGVsbG8=")

    def test_collect_session_mirror_items_preserves_tool_file_outputs(self) -> None:
        events: list[JsonEvent] = [
            {
                "timestamp": "1",
                "type": "response_item",
                "payload": {
                    "type": "function_call_output",
                    "output": [
                        {
                            "type": "input_file",
                            "filename": "report.txt",
                            "file_data": "data:text/plain;base64,aGVsbG8=",
                        }
                    ],
                },
            }
        ]

        items = _collect_items(events)

        self.assertEqual(len(items), 1)
        self.assertEqual(items[0]["kind"], "file")
        self.assertEqual(items[0]["role"], "assistant")
        self.assertEqual(items[0]["phase"], "tool_file")
        self.assertEqual(items[0]["text"], "Codex file output: report.txt")
        self.assertEqual(items[0]["attachment_url"], "data:text/plain;base64,aGVsbG8=")
        self.assertEqual(items[0]["attachment_filename"], "report.txt")

    def test_collect_session_mirror_items_preserves_tool_file_path_outputs(self) -> None:
        with TemporaryDirectory() as temp_dir:
            output_root = Path(temp_dir)
            attachment_path = output_root / "report.csv"
            _ = attachment_path.write_bytes(b"hello file")
            events: list[JsonEvent] = [
                {
                    "timestamp": "1",
                    "type": "response_item",
                    "payload": {
                        "type": "function_call_output",
                        "output": [
                            {
                                "type": "output_file",
                                "path": str(attachment_path),
                            }
                        ],
                    },
                }
            ]

            with mock.patch.object(
                delivery_runtime,
                "CODEX_SESSION_MIRROR_ATTACHMENT_DIR",
                output_root,
            ):
                items = _collect_items(events)

        self.assertEqual(len(items), 1)
        self.assertEqual(items[0]["kind"], "file")
        self.assertEqual(items[0]["attachment_url"], str(attachment_path))
        self.assertEqual(items[0]["attachment_filename"], "report.csv")

    def test_collect_session_mirror_items_skips_unsafe_tool_file_path_outputs(self) -> None:
        with TemporaryDirectory() as temp_dir:
            attachment_path = Path(temp_dir) / "report.csv"
            _ = attachment_path.write_bytes(b"hello file")
            events: list[JsonEvent] = [
                {
                    "timestamp": "1",
                    "type": "response_item",
                    "payload": {
                        "type": "function_call_output",
                        "output": [
                            {
                                "type": "output_file",
                                "path": str(attachment_path),
                            }
                        ],
                    },
                }
            ]

            items = _collect_items(events)

        self.assertEqual(items, [])

    def test_collect_session_mirror_items_sanitizes_tool_file_filename(self) -> None:
        events: list[JsonEvent] = [
            {
                "timestamp": "1",
                "type": "response_item",
                "payload": {
                    "type": "function_call_output",
                    "output": [
                        {
                            "type": "file",
                            "filename": "../bad\nname?.txt",
                            "file_data": "data:text/plain;base64,aGVsbG8=",
                        }
                    ],
                },
            }
        ]

        items = _collect_items(events)

        self.assertEqual(len(items), 1)
        self.assertEqual(items[0]["text"], "Codex file output: bad_name_.txt")
        self.assertEqual(items[0]["attachment_filename"], "bad_name_.txt")

    def test_collect_session_mirror_items_skips_remote_file_urls(self) -> None:
        events: list[JsonEvent] = [
            {
                "timestamp": "1",
                "type": "response_item",
                "payload": {
                    "type": "function_call_output",
                    "output": [
                        {
                            "type": "output_file",
                            "filename": "remote.txt",
                            "file_url": "https://example.com/remote.txt",
                        }
                    ],
                },
            }
        ]

        items = _collect_items(events)

        self.assertEqual(items, [])
