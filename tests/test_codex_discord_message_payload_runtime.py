from __future__ import annotations

import tempfile
import unittest
from dataclasses import dataclass
from pathlib import Path

import codex_discord_empty_content_notice as discord_empty_content_notice
import codex_discord_message_payload_runtime as payload_runtime


@dataclass(frozen=True, slots=True)
class FakeChannel:
    id: int | None


@dataclass(frozen=True, slots=True)
class FakeMessage:
    channel: FakeChannel
    attachments: tuple[()] = ()
    embeds: tuple[()] = ()
    stickers: tuple[()] = ()


class MessagePayloadRuntimeTests(unittest.IsolatedAsyncioTestCase):
    async def test_build_prompt_without_attachments_returns_prompt(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            runtime = payload_runtime.MessagePayloadRuntime(
                attachment_download_dir=Path(temp_dir),
                get_message_id=lambda message: 123,
                message_has_non_text_payload=lambda message: False,
                send_chunks=self._send_chunks,
                log=self._log,
                attachments_enabled=lambda: True,
                get_attachment_max_bytes=lambda: 100,
                get_attachment_text_inline_max_bytes=lambda: 10,
            )

            prompt = await runtime.build_prompt_with_discord_attachments(
                FakeMessage(FakeChannel(10)),
                "hello",
            )

            self.assertEqual(prompt, "hello")

    async def test_empty_content_notice_sends_to_channel(self) -> None:
        sent: list[tuple[int | None, str]] = []
        logs: list[str] = []

        async def send_chunks(
            channel: discord_empty_content_notice.EmptyContentChannel,
            text: str,
        ) -> int:
            sent.append((channel.id, text))
            return 1

        runtime = payload_runtime.MessagePayloadRuntime(
            attachment_download_dir=Path("unused"),
            get_message_id=lambda message: 123,
            message_has_non_text_payload=lambda message: False,
            send_chunks=send_chunks,
            log=logs.append,
        )

        await runtime.maybe_send_empty_content_notice(FakeMessage(FakeChannel(99)))

        self.assertEqual(len(sent), 1)
        self.assertEqual(sent[0][0], 99)
        self.assertTrue(any("empty_content_notice_sent chat=99" in line for line in logs))

    async def _send_chunks(
        self,
        channel: discord_empty_content_notice.EmptyContentChannel,
        text: str,
    ) -> int:
        _ = (channel, text)
        return 1

    def _log(self, message: str) -> None:
        _ = message


if __name__ == "__main__":
    _ = unittest.main()
