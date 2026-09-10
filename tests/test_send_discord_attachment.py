from __future__ import annotations

import json
import os
import tempfile
import unittest
from pathlib import Path
from types import TracebackType
from unittest.mock import patch

import aiohttp

import send_discord_attachment as sender


class FakeResponse:
    def __init__(self, *, status: int, text: str) -> None:
        self.status: int = status
        self._text: str = text

    async def __aenter__(self) -> FakeResponse:
        return self

    async def __aexit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        traceback: TracebackType | None,
    ) -> bool:
        _ = (exc_type, exc, traceback)
        return False

    async def text(self) -> str:
        return self._text


class FakeClientSession:
    def __init__(self, response: FakeResponse, *, get_response: FakeResponse | None = None) -> None:
        self.response: FakeResponse = response
        self.get_response: FakeResponse = get_response or FakeResponse(status=200, text="{}")
        self.posts: list[tuple[str, dict[str, str], aiohttp.FormData]] = []
        self.gets: list[tuple[str, dict[str, str]]] = []

    async def __aenter__(self) -> FakeClientSession:
        return self

    async def __aexit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        traceback: TracebackType | None,
    ) -> bool:
        _ = (exc_type, exc, traceback)
        return False

    def post(self, url: str, *, headers: dict[str, str], data: aiohttp.FormData) -> FakeResponse:
        self.posts.append((url, headers, data))
        return self.response

    def get(self, url: str, *, headers: dict[str, str]) -> FakeResponse:
        self.gets.append((url, headers))
        return self.get_response


def attachment_args(
    *,
    channel_id: str | None = "123",
    thread_ref: str | None = None,
    work_thread: str | None = None,
    content: str = "",
    content_file: str | None = None,
    files: list[str] | None = None,
) -> sender.AttachmentCliArgs:
    return sender.AttachmentCliArgs(
        channel_id=channel_id,
        thread_ref=thread_ref,
        work_thread=work_thread,
        content=content,
        content_file=content_file,
        files=files or [],
    )


class SendDiscordAttachmentTests(unittest.IsolatedAsyncioTestCase):
    def test_load_env_strips_quotes_and_ignores_non_assignments(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            env_path = Path(temp_dir) / ".env"
            _ = env_path.write_text(
                "\n".join(
                    [
                        "# ignored",
                        "DISCORD_BOT_TOKEN = \" token \"",
                        "NO_EQUALS",
                        "OTHER=value",
                    ],
                ),
                encoding="utf-8",
            )

            values = sender.load_env(env_path)

        self.assertEqual(values["DISCORD_BOT_TOKEN"], " token ")
        self.assertEqual(values["OTHER"], "value")
        self.assertNotIn("NO_EQUALS", values)

    def test_get_message_content_prefers_content_file(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            content_path = Path(temp_dir) / "message.txt"
            _ = content_path.write_text(" file content \n", encoding="utf-8")

            content = sender.get_message_content(
                attachment_args(content="inline", content_file=str(content_path)),
            )

        self.assertEqual(content, "file content")

    def test_parse_args_accepts_thread_ref_target(self) -> None:
        with patch("sys.argv", ["send_discord_attachment.py", "--thread-ref", "repo:2", "note.txt"]):
            args = sender.parse_args()

        self.assertIsNone(args.channel_id)
        self.assertEqual(args.thread_ref, "repo:2")
        self.assertIsNone(args.work_thread)
        self.assertEqual(args.files, ["note.txt"])

    def test_parse_args_rejects_multiple_targets(self) -> None:
        with patch(
            "sys.argv",
            [
                "send_discord_attachment.py",
                "--channel-id",
                "123",
                "--work-thread",
                "thread-1",
                "note.txt",
            ],
        ):
            with self.assertRaises(SystemExit):
                _ = sender.parse_args()

    async def test_missing_token_raises_typed_error_before_file_io(self) -> None:
        with patch.dict(os.environ, {}, clear=True):
            with patch.object(sender, "load_env", return_value={}):
                with self.assertRaises(sender.MissingDiscordBotTokenError):
                    _ = await sender.send_discord_attachment(attachment_args(files=["missing.txt"]))

    async def test_missing_files_raise_file_not_found_before_network(self) -> None:
        with patch.dict(os.environ, {"DISCORD_BOT_TOKEN": "token"}, clear=True):
            with self.assertRaisesRegex(FileNotFoundError, "missing.txt"):
                _ = await sender.send_discord_attachment(attachment_args(files=["missing.txt"]))

    async def test_http_failure_raises_typed_error_with_truncated_response_text(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            attachment_path = Path(temp_dir) / "note.txt"
            _ = attachment_path.write_text("payload", encoding="utf-8")
            response = FakeResponse(status=500, text="x" * 1200)
            fake_session = FakeClientSession(response)

            with patch.dict(os.environ, {"DISCORD_BOT_TOKEN": "token"}, clear=True):
                with patch.object(sender, "ClientSession", return_value=fake_session):
                    with self.assertRaises(sender.DiscordSendFailedError) as caught:
                        _ = await sender.send_discord_attachment(attachment_args(files=[str(attachment_path)]))

        self.assertEqual(caught.exception.status, 500)
        self.assertEqual(caught.exception.response_text, "x" * 1000)
        self.assertEqual(fake_session.gets[0][0], "https://discord.com/api/v10/channels/123")
        self.assertEqual(fake_session.posts[0][1], {"Authorization": "Bot token"})

    async def test_unknown_direct_channel_fails_before_upload(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            attachment_path = Path(temp_dir) / "note.txt"
            _ = attachment_path.write_text("payload", encoding="utf-8")
            fake_session = FakeClientSession(
                FakeResponse(status=200, text="{}"),
                get_response=FakeResponse(status=404, text='{"message":"Unknown Channel"}'),
            )

            with patch.dict(os.environ, {"DISCORD_BOT_TOKEN": "token"}, clear=True):
                with patch.object(sender, "ClientSession", return_value=fake_session):
                    with self.assertRaisesRegex(sender.DiscordChannelAccessError, "Unknown Channel"):
                        _ = await sender.send_discord_attachment(attachment_args(files=[str(attachment_path)]))

        self.assertEqual(fake_session.posts, [])

    async def test_forbidden_direct_channel_fails_before_upload(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            attachment_path = Path(temp_dir) / "note.txt"
            _ = attachment_path.write_text("payload", encoding="utf-8")
            fake_session = FakeClientSession(
                FakeResponse(status=200, text="{}"),
                get_response=FakeResponse(status=403, text='{"message":"Forbidden"}'),
            )

            with patch.dict(os.environ, {"DISCORD_BOT_TOKEN": "token"}, clear=True):
                with patch.object(sender, "ClientSession", return_value=fake_session):
                    with self.assertRaisesRegex(sender.DiscordChannelAccessError, "Forbidden"):
                        _ = await sender.send_discord_attachment(attachment_args(files=[str(attachment_path)]))

        self.assertEqual(fake_session.posts, [])

    async def test_unknown_work_thread_reports_active_archived_text(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            attachment_path = Path(temp_dir) / "note.txt"
            _ = attachment_path.write_text("payload", encoding="utf-8")

            with patch.dict(os.environ, {"DISCORD_BOT_TOKEN": "token"}, clear=True):
                with patch.object(sender.thread_store, "resolve_thread_ref", side_effect=sender.thread_store.ThreadStoreError("missing")):
                    with patch.object(sender.thread_store, "resolve_archived_thread_ref", side_effect=sender.thread_store.ThreadStoreError("missing")):
                        with self.assertRaisesRegex(sender.AttachmentTargetError, "not in active/archived threads"):
                            _ = await sender.send_discord_attachment(
                                attachment_args(
                                    channel_id=None,
                                    work_thread="missing",
                                    files=[str(attachment_path)],
                                ),
                            )

    async def test_success_returns_decoded_json_without_live_network(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            attachment_path = Path(temp_dir) / "note.txt"
            _ = attachment_path.write_text("payload", encoding="utf-8")
            response_body = {"id": "m1", "channel_id": "123", "attachments": [{"filename": "note.txt"}]}
            response = FakeResponse(status=200, text=json.dumps(response_body))
            fake_session = FakeClientSession(response)

            with patch.dict(os.environ, {"DISCORD_BOT_TOKEN": "token"}, clear=True):
                with patch.object(sender, "ClientSession", return_value=fake_session):
                    result = await sender.send_discord_attachment(
                        attachment_args(channel_id="123", content="hello", files=[str(attachment_path)]),
                    )

        self.assertEqual(result, response_body)
        self.assertEqual(fake_session.gets[0][0], "https://discord.com/api/v10/channels/123")
        self.assertEqual(fake_session.posts[0][0], "https://discord.com/api/v10/channels/123/messages")


if __name__ == "__main__":
    _ = unittest.main()
