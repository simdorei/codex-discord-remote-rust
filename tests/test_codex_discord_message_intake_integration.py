from __future__ import annotations

from collections.abc import Awaitable, Callable
from pathlib import Path
from types import SimpleNamespace
from typing import Protocol, cast
from unittest import mock
import tempfile
import unittest

import codex_discord_bot as bot

from tests.test_codex_discord_bot import EnvPatch, FakeAttachment, FakeMessage


class MessageChannelLike(Protocol):
    id: int


class MessageWithChannel(Protocol):
    channel: MessageChannelLike


OnMessageFunc = Callable[[bot.CodexDiscordBot, MessageWithChannel], Awaitable[None]]
MaybeSendEmptyContentNoticeFunc = Callable[[MessageWithChannel], Awaitable[None]]
BuildPromptWithDiscordAttachmentsFunc = Callable[[MessageWithChannel, str], Awaitable[str]]


class ProcessDiscordMessageFunc(Protocol):
    def __call__(
        self,
        client: bot.CodexDiscordBot,
        message: MessageWithChannel,
        *,
        source: str,
    ) -> Awaitable[None]: ...


def _client(
    *,
    plain_ask_mention_user_ids: set[int] | None = None,
) -> bot.CodexDiscordBot:
    def is_allowed_message_channel(channel: MessageChannelLike | None) -> bool:
        _ = channel
        return True

    def is_allowed_user(user_id: int | None) -> bool:
        _ = user_id
        return True

    return cast(
        bot.CodexDiscordBot,
        cast(
            object,
            SimpleNamespace(
                enable_prefix_commands=True,
                plain_ask_mention_user_ids=set() if plain_ask_mention_user_ids is None else plain_ask_mention_user_ids,
                is_allowed_message_channel=is_allowed_message_channel,
                is_allowed_user=is_allowed_user,
            ),
        ),
    )


def _discord_message(message: FakeMessage) -> MessageWithChannel:
    return cast(MessageWithChannel, cast(object, message))


async def _run_on_message(client: bot.CodexDiscordBot, message: FakeMessage) -> None:
    on_message = cast(OnMessageFunc, bot.CodexDiscordBot.on_message)
    await on_message(client, _discord_message(message))


async def _run_process_discord_message(client: bot.CodexDiscordBot, message: FakeMessage, *, source: str) -> None:
    process_discord_message = cast(ProcessDiscordMessageFunc, bot.CodexDiscordBot.process_discord_message)
    await process_discord_message(client, _discord_message(message), source=source)


async def _maybe_send_empty_content_notice(message: FakeMessage) -> None:
    maybe_send_empty_content_notice = cast(MaybeSendEmptyContentNoticeFunc, bot.maybe_send_empty_content_notice)
    await maybe_send_empty_content_notice(_discord_message(message))


async def _build_prompt_with_discord_attachments(message: FakeMessage, prompt: str) -> str:
    build_prompt = cast(BuildPromptWithDiscordAttachmentsFunc, bot.build_prompt_with_discord_attachments)
    return await build_prompt(_discord_message(message), prompt)


class DiscordMessageIntakeIntegrationTests(unittest.IsolatedAsyncioTestCase):
    async def test_on_message_logs_received_before_empty_content_ignore(self) -> None:
        message = FakeMessage(content="", channel_id=333)
        bot.EMPTY_CONTENT_NOTICE_LAST_SENT.clear()

        with tempfile.TemporaryDirectory() as temp_dir:
            log_path = Path(temp_dir) / "discord-smoke.log"
            with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                await _run_on_message(_client(), message)
            log_text = log_path.read_text(encoding="utf-8")

        self.assertEqual(len(message.channel.messages), 1)
        self.assertIn("Discord did not provide the text content", message.channel.messages[0][0])
        self.assertIn("message_received chat=333", log_text)
        self.assertIn("content_len=0", log_text)
        self.assertIn("ignored_message reason=empty_content chat=333", log_text)
        self.assertIn("empty_content_notice_sent chat=333", log_text)

    async def test_empty_content_notice_uses_channel_cooldown(self) -> None:
        bot.EMPTY_CONTENT_NOTICE_LAST_SENT.clear()
        first = FakeMessage(content="", channel_id=333)
        second = FakeMessage(content="", channel_id=333)

        with tempfile.TemporaryDirectory() as temp_dir:
            log_path = Path(temp_dir) / "discord-smoke.log"
            with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                await _maybe_send_empty_content_notice(first)
                await _maybe_send_empty_content_notice(second)
            log_text = log_path.read_text(encoding="utf-8")

        self.assertEqual(len(first.channel.messages), 1)
        self.assertEqual(second.channel.messages, [])
        self.assertIn("empty_content_notice_sent chat=333", log_text)
        self.assertIn("empty_content_notice_skipped reason=cooldown chat=333", log_text)

    async def test_empty_content_notice_skips_non_text_payload(self) -> None:
        bot.EMPTY_CONTENT_NOTICE_LAST_SENT.clear()
        message = FakeMessage(content="", channel_id=333)
        message.attachments = [object()]

        with tempfile.TemporaryDirectory() as temp_dir:
            log_path = Path(temp_dir) / "discord-smoke.log"
            with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                await _maybe_send_empty_content_notice(message)
            log_text = log_path.read_text(encoding="utf-8")

        self.assertEqual(message.channel.messages, [])
        self.assertIn("empty_content_notice_skipped reason=non_text_payload chat=333", log_text)

    async def test_on_message_attachment_only_routes_saved_text_file_to_plain_ask(self) -> None:
        original_attachment_dir = bot.ATTACHMENT_DOWNLOAD_DIR
        calls: list[tuple[MessageWithChannel, str, str | None]] = []
        try:
            def fake_get_mirrored_codex_thread_id(channel_id: int | None) -> str:
                _ = channel_id
                return "thread-1"

            async def fake_handle_plain_ask(
                message: MessageWithChannel,
                prompt: str,
                *,
                target_thread_id: str | None = None,
            ) -> None:
                calls.append((message, prompt, target_thread_id))
            message = FakeMessage(content="", channel_id=333, message_id=1234)
            message.attachments = [
                FakeAttachment("note.txt", b"hello from a text file", content_type="text/plain")
            ]

            with tempfile.TemporaryDirectory() as temp_dir:
                bot.ATTACHMENT_DOWNLOAD_DIR = Path(temp_dir) / "attachments"
                log_path = Path(temp_dir) / "discord-smoke.log"
                with (
                    EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)),
                    mock.patch.object(
                        bot,
                        "get_mirrored_codex_thread_id",
                        side_effect=fake_get_mirrored_codex_thread_id,
                    ),
                    mock.patch.object(bot, "handle_plain_ask", side_effect=fake_handle_plain_ask),
                ):
                    await _run_process_discord_message(_client(), message, source="test")
                log_text = log_path.read_text(encoding="utf-8")

            self.assertEqual(len(calls), 1)
            _message, prompt, target_thread_id = calls[0]
            self.assertEqual(target_thread_id, "thread-1")
            self.assertIn("Please inspect the attached Discord file(s).", prompt)
            self.assertIn("note.txt", prompt)
            self.assertIn("hello from a text file", prompt)
            self.assertIn("attachment_saved message=1234 filename=note.txt", log_text)
        finally:
            bot.ATTACHMENT_DOWNLOAD_DIR = original_attachment_dir

    async def test_on_message_persists_discord_thread_id_for_known_mirror_target(self) -> None:
        persisted: list[tuple[str, int]] = []
        handled: list[tuple[str | None, int]] = []

        def fake_get_mirrored_codex_thread_id(channel_id: int | None) -> str | None:
            _ = channel_id
            return "thread-1"

        def fake_describe_mirrored_project_channel(channel_id: int | None) -> None:
            _ = channel_id

        def fake_persist_inbound_mirror_thread_channel(target_thread_id: str, discord_thread_id: int) -> None:
            persisted.append((target_thread_id, discord_thread_id))

        async def fake_handle_plain_ask(
            message: MessageWithChannel,
            prompt: str,
            *,
            target_thread_id: str | None = None,
        ) -> None:
            _ = prompt
            handled.append((target_thread_id, int(getattr(message.channel, "id", 0))))

        message = FakeMessage(content="please run", channel_id=333)
        message.channel.parent_id = 111

        with tempfile.TemporaryDirectory() as temp_dir:
            log_path = Path(temp_dir) / "discord-smoke.log"
            with (
                EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)),
                EnvPatch("DISCORD_PLAIN_ASK_CONTEXT_FALLBACK", "0"),
                mock.patch.object(
                    bot,
                    "get_mirrored_codex_thread_id",
                    side_effect=fake_get_mirrored_codex_thread_id,
                ),
                mock.patch.object(
                    bot,
                    "describe_mirrored_project_channel",
                    side_effect=fake_describe_mirrored_project_channel,
                ),
                mock.patch.object(
                    bot,
                    "persist_inbound_mirror_thread_channel",
                    side_effect=fake_persist_inbound_mirror_thread_channel,
                ),
                mock.patch.object(bot, "handle_plain_ask", side_effect=fake_handle_plain_ask),
            ):
                await _run_on_message(_client(plain_ask_mention_user_ids={1500506752234422322}), message)
            log_text = log_path.read_text(encoding="utf-8")

        self.assertEqual(persisted, [("thread-1", 333)])
        self.assertEqual(handled, [("thread-1", 333)])
        self.assertIn("inbound_mirror_channel_persisted target=thread-1 channel=333", log_text)

    async def test_build_prompt_with_discord_attachments_saves_image_path_without_preview(self) -> None:
        original_attachment_dir = bot.ATTACHMENT_DOWNLOAD_DIR
        try:
            message = FakeMessage(content="look at this", channel_id=333, message_id=1235)
            message.attachments = [
                FakeAttachment("screen.png", b"\x89PNG\r\n\x1a\n", content_type="image/png")
            ]
            with tempfile.TemporaryDirectory() as temp_dir:
                bot.ATTACHMENT_DOWNLOAD_DIR = Path(temp_dir) / "attachments"
                prompt = await _build_prompt_with_discord_attachments(message, "look at this")

            self.assertIn("look at this", prompt)
            self.assertIn("screen.png", prompt)
            self.assertIn("content_type: image/png", prompt)
            self.assertNotIn("Attachment text previews:", prompt)
        finally:
            bot.ATTACHMENT_DOWNLOAD_DIR = original_attachment_dir


if __name__ == "__main__":
    _ = unittest.main()
