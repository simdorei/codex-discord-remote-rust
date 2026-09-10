from __future__ import annotations

import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import send_discord_attachment as sender
from tests.test_send_discord_attachment import FakeClientSession, FakeResponse, attachment_args


class SendDiscordAttachmentThreadRefTests(unittest.IsolatedAsyncioTestCase):
    async def test_thread_ref_resolves_mirror_channel_and_validates_access(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            attachment_path = Path(temp_dir) / "note.txt"
            _ = attachment_path.write_text("payload", encoding="utf-8")
            response_body = {"id": "m1", "channel_id": "555", "attachments": [{"filename": "note.txt"}]}
            fake_session = FakeClientSession(FakeResponse(status=200, text=json.dumps(response_body)))
            thread = sender.ThreadInfo(
                id="thread-1",
                title="Task",
                cwd=str(Path(temp_dir)),
                updated_at=1,
                rollout_path="rollout.jsonl",
                model="gpt",
                reasoning_effort="",
                tokens_used=0,
            )

            with patch.dict(os.environ, {"DISCORD_BOT_TOKEN": "token"}, clear=True):
                with patch.object(sender, "ClientSession", return_value=fake_session):
                    with patch.object(sender.thread_store, "resolve_thread_ref", return_value=thread):
                        with patch.object(sender.discord_store, "get_mirror_thread_row_by_codex_thread_id", return_value=(111, 555)):
                            result = await sender.send_discord_attachment(
                                attachment_args(
                                    channel_id=None,
                                    thread_ref="repo:2",
                                    files=[str(attachment_path)],
                                ),
                            )

        self.assertEqual(result, response_body)
        self.assertEqual(fake_session.gets[0][0], "https://discord.com/api/v10/channels/555")
        self.assertEqual(fake_session.posts[0][0], "https://discord.com/api/v10/channels/555/messages")

    async def test_thread_ref_stale_mirror_mapping_reports_recovery_text(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            attachment_path = Path(temp_dir) / "note.txt"
            _ = attachment_path.write_text("payload", encoding="utf-8")
            fake_session = FakeClientSession(
                FakeResponse(status=200, text="{}"),
                get_response=FakeResponse(status=404, text='{"message":"Unknown Channel"}'),
            )
            thread = sender.ThreadInfo(
                id="thread-1",
                title="Task",
                cwd=str(Path(temp_dir)),
                updated_at=1,
                rollout_path="rollout.jsonl",
                model="gpt",
                reasoning_effort="",
                tokens_used=0,
            )

            with patch.dict(os.environ, {"DISCORD_BOT_TOKEN": "token"}, clear=True):
                with patch.object(sender, "ClientSession", return_value=fake_session):
                    with patch.object(sender.thread_store, "resolve_thread_ref", return_value=thread):
                        with patch.object(sender.discord_store, "get_mirror_thread_row_by_codex_thread_id", return_value=(111, 555)):
                            with self.assertRaisesRegex(
                                sender.DiscordChannelAccessError,
                                "mirror mapping stale: run !mirror check, then !mirror sync",
                            ):
                                _ = await sender.send_discord_attachment(
                                    attachment_args(
                                        channel_id=None,
                                        thread_ref="repo:2",
                                        files=[str(attachment_path)],
                                    ),
                                )

        self.assertEqual(fake_session.posts, [])


if __name__ == "__main__":
    _ = unittest.main()
