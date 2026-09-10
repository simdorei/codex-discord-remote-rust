from __future__ import annotations

import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import codex_discord_attachment_prompt as attachment_prompt
import codex_discord_attachments as attachments


class FakeAttachment:
    def __init__(
        self,
        filename: str,
        data: bytes,
        *,
        content_type: str = "application/octet-stream",
    ) -> None:
        self.filename: str = filename
        self._data: bytes = data
        self.content_type: str = content_type
        self.size: int = len(data)

    async def save(self, destination: Path) -> None:
        _ = destination.write_bytes(self._data)


class FailingSaveAttachment:
    filename: str = "broken.txt"
    content_type: str = "text/plain"
    size: int = 4

    async def save(self, _destination: Path) -> None:
        raise OSError("disk unavailable")


class FakeChannel:
    def __init__(self, channel_id: int) -> None:
        self.id: int = channel_id


class FakeMessage:
    def __init__(self, message_id: int, attachment_list: list[FakeAttachment | FailingSaveAttachment]) -> None:
        self.id: int = message_id
        self.channel: FakeChannel = FakeChannel(333)
        self.attachments: list[FakeAttachment | FailingSaveAttachment] = attachment_list
        self.embeds: list[str] = []
        self.stickers: list[str] = []


class DiscordAttachmentConfigTests(unittest.TestCase):
    def test_attachment_config_uses_existing_defaults(self) -> None:
        with patch.dict(os.environ, {}, clear=True):
            self.assertTrue(attachments.discord_attachments_enabled())
            self.assertEqual(attachments.get_discord_attachment_max_bytes(), 25 * 1024 * 1024)
            self.assertEqual(attachments.get_discord_attachment_text_inline_max_bytes(), 32 * 1024)

    def test_attachment_config_clamps_env_values(self) -> None:
        with patch.dict(
            os.environ,
            {
                "DISCORD_ENABLE_ATTACHMENTS": "false",
                "DISCORD_ATTACHMENT_MAX_BYTES": str(200 * 1024 * 1024),
                "DISCORD_ATTACHMENT_TEXT_INLINE_MAX_BYTES": "-1",
            },
            clear=True,
        ):
            self.assertFalse(attachments.discord_attachments_enabled())
            self.assertEqual(attachments.get_discord_attachment_max_bytes(), 100 * 1024 * 1024)
            self.assertEqual(attachments.get_discord_attachment_text_inline_max_bytes(), 0)


class DiscordAttachmentPromptTests(unittest.TestCase):
    def test_attachment_prompt_renders_saved_details_and_text_previews(self) -> None:
        details = [
            attachment_prompt.render_saved_attachment_detail(
                index=1,
                filename="note.txt",
                destination=Path("attachments") / "01-note.txt",
                content_type="text/plain",
                size_bytes=17,
            ),
            "2. large.log skipped: file is 6 bytes; limit is 5 bytes.",
        ]
        previews = [
            attachment_prompt.AttachmentTextPreview(
                filename="note.txt",
                preview="hello from a text file",
            )
        ]

        prompt = attachment_prompt.render_attachment_prompt("inspect", details, previews)

        self.assertEqual(
            prompt,
            "\n".join(
                [
                    "inspect",
                    "",
                    "Discord attachments saved locally:",
                    "1. note.txt",
                    f"   path: {Path('attachments') / '01-note.txt'}",
                    "   content_type: text/plain",
                    "   size_bytes: 17",
                    "2. large.log skipped: file is 6 bytes; limit is 5 bytes.",
                    "",
                    "Attachment text previews:",
                    "--- note.txt ---",
                    "```text",
                    "hello from a text file",
                    "```",
                ]
            ),
        )

    def test_attachment_prompt_returns_base_prompt_without_details(self) -> None:
        prompt = attachment_prompt.render_attachment_prompt(
            "inspect",
            [],
            [attachment_prompt.AttachmentTextPreview(filename="ignored.txt", preview="ignored")],
        )

        self.assertEqual(prompt, "inspect")


class DiscordAttachmentBuilderEdgeTests(unittest.IsolatedAsyncioTestCase):
    async def test_process_prompt_attachment_returns_saved_detail_and_preview(self) -> None:
        message = FakeMessage(
            1238,
            [FakeAttachment("note.txt", b"hello", content_type="text/plain")],
        )
        log_lines: list[str] = []

        with tempfile.TemporaryDirectory() as temp_dir:
            prompt = await attachments.build_prompt_with_discord_attachments(
                message,
                "inspect",
                attachments_enabled=True,
                max_bytes=100,
                text_inline_max_bytes=100,
                attachment_download_dir=Path(temp_dir),
                log_func=log_lines.append,
                get_message_id=lambda _candidate: message.id,
            )

        self.assertIn("1. note.txt", prompt)
        self.assertIn("content_type: text/plain", prompt)
        self.assertIn("size_bytes: 5", prompt)
        self.assertIn("--- note.txt ---", prompt)
        self.assertIn("hello", prompt)
        self.assertTrue(any("attachment_saved message=1238 filename=note.txt" in line for line in log_lines))

    async def test_builder_logs_save_failure(self) -> None:
        message = FakeMessage(1236, [FailingSaveAttachment()])
        log_lines: list[str] = []

        with tempfile.TemporaryDirectory() as temp_dir:
            prompt = await attachments.build_prompt_with_discord_attachments(
                message,
                "inspect",
                attachments_enabled=True,
                max_bytes=100,
                text_inline_max_bytes=100,
                attachment_download_dir=Path(temp_dir),
                log_func=log_lines.append,
                get_message_id=lambda candidate: getattr(candidate, "id", None),
            )

        self.assertIn("1. broken.txt failed to save: OSError.", prompt)
        self.assertTrue(any("attachment_save_failed message=1236" in line for line in log_lines))
        self.assertNotIn("Attachment text previews:", prompt)

    async def test_builder_logs_preview_failure_without_dropping_saved_detail(self) -> None:
        message = FakeMessage(
            1237,
            [FakeAttachment("note.txt", b"hello", content_type="text/plain")],
        )
        log_lines: list[str] = []

        with tempfile.TemporaryDirectory() as temp_dir:
            with patch.object(
                attachments,
                "read_attachment_text_preview",
                side_effect=OSError("cannot read"),
            ):
                prompt = await attachments.build_prompt_with_discord_attachments(
                    message,
                    "inspect",
                    attachments_enabled=True,
                    max_bytes=100,
                    text_inline_max_bytes=100,
                    attachment_download_dir=Path(temp_dir),
                    log_func=log_lines.append,
                    get_message_id=lambda candidate: getattr(candidate, "id", None),
                )

        self.assertIn("1. note.txt", prompt)
        self.assertIn("content_type: text/plain", prompt)
        self.assertTrue(
            any("attachment_preview_failed message=1237" in line for line in log_lines)
        )
        self.assertNotIn("Attachment text previews:", prompt)
