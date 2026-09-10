from __future__ import annotations

import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from typing import cast
from unittest.mock import patch

import codex_discord_attachments as attachments
import codex_discord_message_gate as message_gate


def _mention_message(message: SimpleNamespace) -> message_gate.MessageWithMentions:
    return cast(message_gate.MessageWithMentions, cast(object, message))


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


class DiscordMessageIntakeTests(unittest.IsolatedAsyncioTestCase):
    def test_message_mentions_other_bot_ignores_required_bridge_user(self) -> None:
        message = SimpleNamespace(
            mentions=[
                SimpleNamespace(id=1511380398914142379, bot=True),
                SimpleNamespace(id=1500506752234422322, bot=True),
            ],
        )

        self.assertTrue(
            message_gate.message_mentions_other_bot(
                _mention_message(message),
                {1511380398914142379},
            )
        )

    def test_prepare_plain_ask_content_strips_required_mention(self) -> None:
        message = SimpleNamespace(raw_mentions=[1500506752234422322], mentions=[])

        result = message_gate.prepare_plain_ask_content(
            _mention_message(message),
            "<@1500506752234422322> please run",
            {1500506752234422322},
            target_thread_id=None,
            has_attachments=False,
        )

        self.assertEqual(result.action, message_gate.PlainAskGateAction.ACCEPT)
        self.assertEqual(result.content, "please run")
        self.assertTrue(result.matched_mention)
        self.assertFalse(result.context_fallback)

    def test_prepare_plain_ask_content_allows_mirrored_thread_without_required_mention(
        self,
    ) -> None:
        message = SimpleNamespace(raw_mentions=[], mentions=[])

        result = message_gate.prepare_plain_ask_content(
            _mention_message(message),
            "짧은 확인",
            {1500506752234422322},
            target_thread_id="thread-1",
            has_attachments=False,
        )

        self.assertEqual(result.action, message_gate.PlainAskGateAction.ACCEPT)
        self.assertEqual(result.content, "짧은 확인")
        self.assertFalse(result.matched_mention)

    def test_prepare_plain_ask_content_accepts_context_fallback(self) -> None:
        message = SimpleNamespace(raw_mentions=[], mentions=[])

        with patch.dict(
            "os.environ",
            {
                "DISCORD_PLAIN_ASK_CONTEXT_FALLBACK": "1",
                "DISCORD_PLAIN_ASK_CONTEXT_KEYWORDS": message_gate.DEFAULT_PLAIN_ASK_CONTEXT_KEYWORDS,
            },
        ):
            result = message_gate.prepare_plain_ask_content(
                _mention_message(message),
                "codex explain this in Korean",
                {1500506752234422322},
                target_thread_id=None,
                has_attachments=False,
            )

        self.assertEqual(result.action, message_gate.PlainAskGateAction.ACCEPT)
        self.assertEqual(result.content, "codex explain this in Korean")
        self.assertTrue(result.matched_mention)
        self.assertTrue(result.context_fallback)

    def test_prepare_plain_ask_content_rejects_missing_required_mention(self) -> None:
        message = SimpleNamespace(raw_mentions=[], mentions=[])

        result = message_gate.prepare_plain_ask_content(
            _mention_message(message),
            "plain channel chatter",
            {1500506752234422322},
            target_thread_id=None,
            has_attachments=False,
        )

        self.assertEqual(
            result.action,
            message_gate.PlainAskGateAction.REQUIRED_MENTION_MISSING,
        )
        self.assertEqual(result.content, "plain channel chatter")
        self.assertFalse(result.matched_mention)

    def test_prepare_plain_ask_content_rejects_mention_only_prompt(self) -> None:
        message = SimpleNamespace(raw_mentions=[1500506752234422322], mentions=[])

        result = message_gate.prepare_plain_ask_content(
            _mention_message(message),
            "<@!1500506752234422322>",
            {1500506752234422322},
            target_thread_id=None,
            has_attachments=False,
        )

        self.assertEqual(result.action, message_gate.PlainAskGateAction.MENTION_ONLY_CONTENT)
        self.assertEqual(result.content, "")
        self.assertTrue(result.matched_mention)

    def test_prepare_plain_ask_content_accepts_mention_only_attachment_prompt(self) -> None:
        message = SimpleNamespace(raw_mentions=[1500506752234422322], mentions=[])

        result = message_gate.prepare_plain_ask_content(
            _mention_message(message),
            "<@!1500506752234422322>",
            {1500506752234422322},
            target_thread_id=None,
            has_attachments=True,
        )

        self.assertEqual(result.action, message_gate.PlainAskGateAction.ACCEPT)
        self.assertEqual(result.content, message_gate.ATTACHMENT_INSPECTION_PROMPT)
        self.assertTrue(result.matched_mention)

    def test_prepare_plain_ask_content_rejects_other_bot_in_mirrored_thread(self) -> None:
        message = SimpleNamespace(
            raw_mentions=[1500506752234422322],
            mentions=[SimpleNamespace(id=1500506752234422322, bot=True)],
        )

        result = message_gate.prepare_plain_ask_content(
            _mention_message(message),
            "<@1500506752234422322> ping",
            {1511380398914142379},
            target_thread_id="thread-1",
            has_attachments=False,
        )

        self.assertEqual(
            result.action,
            message_gate.PlainAskGateAction.OTHER_BOT_MENTION_IN_MIRRORED_THREAD,
        )
        self.assertFalse(result.matched_mention)

    def test_prepare_bot_bridge_prefix_content_strips_bridge_mention(self) -> None:
        result = message_gate.prepare_bot_bridge_prefix_content(
            "!runners <@1511380398914142379>",
            {1511380398914142379},
        )

        self.assertEqual(result.content, "!runners")
        self.assertTrue(result.stripped_mention)

    def test_prepare_bot_bridge_prefix_content_keeps_unmatched_or_non_prefix(self) -> None:
        unmatched = message_gate.prepare_bot_bridge_prefix_content(
            "!runners <@1511380398914142379>",
            {1500506752234422322},
        )
        non_prefix = message_gate.prepare_bot_bridge_prefix_content(
            "<@1511380398914142379> !runners",
            {1511380398914142379},
        )

        self.assertEqual(unmatched.content, "!runners <@1511380398914142379>")
        self.assertFalse(unmatched.stripped_mention)
        self.assertEqual(non_prefix.content, "<@1511380398914142379> !runners")
        self.assertFalse(non_prefix.stripped_mention)

    async def test_attachments_build_prompt_with_text_preview(self) -> None:
        message = SimpleNamespace(
            id=1234,
            channel=SimpleNamespace(id=333),
            attachments=[
                FakeAttachment("note.txt", b"hello from a text file", content_type="text/plain")
            ],
        )
        log_lines: list[str] = []

        with tempfile.TemporaryDirectory() as temp_dir:
            prompt = await attachments.build_prompt_with_discord_attachments(
                cast(attachments.AttachmentMessageLike, cast(object, message)),
                "",
                attachments_enabled=True,
                max_bytes=1024,
                text_inline_max_bytes=1024,
                attachment_download_dir=Path(temp_dir) / "attachments",
                log_func=log_lines.append,
                get_message_id=lambda candidate: getattr(candidate, "id", None),
            )

        self.assertIn("Please inspect the attached Discord file(s).", prompt)
        self.assertIn("note.txt", prompt)
        self.assertIn("hello from a text file", prompt)
        self.assertTrue(any("attachment_saved message=1234 filename=note.txt" in line for line in log_lines))

    async def test_attachments_skip_large_file_without_saving(self) -> None:
        message = SimpleNamespace(
            id=1235,
            channel=SimpleNamespace(id=333),
            attachments=[FakeAttachment("large.log", b"abcdef", content_type="text/plain")],
        )
        log_lines: list[str] = []

        with tempfile.TemporaryDirectory() as temp_dir:
            prompt = await attachments.build_prompt_with_discord_attachments(
                cast(attachments.AttachmentMessageLike, cast(object, message)),
                "inspect",
                attachments_enabled=True,
                max_bytes=3,
                text_inline_max_bytes=1024,
                attachment_download_dir=Path(temp_dir) / "attachments",
                log_func=log_lines.append,
                get_message_id=lambda candidate: getattr(candidate, "id", None),
            )

        self.assertIn("large.log skipped", prompt)
        self.assertTrue(any("attachment_skipped reason=size message=1235" in line for line in log_lines))


if __name__ == "__main__":
    _ = unittest.main()
