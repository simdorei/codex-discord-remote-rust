from __future__ import annotations

import unittest
from dataclasses import dataclass

import codex_discord_prefix_resume_command as resume_command


@dataclass(frozen=True, slots=True)
class FakeChannel:
    id: int = 123


@dataclass(frozen=True, slots=True)
class FakeAuthor:
    id: int = 456


@dataclass(frozen=True, slots=True)
class FakeMessage:
    channel: FakeChannel = FakeChannel()
    author: FakeAuthor = FakeAuthor()


class PrefixResumeCommandTests(unittest.IsolatedAsyncioTestCase):
    async def test_resume_uses_current_channel_when_ref_is_omitted(self) -> None:
        sent: list[str] = []
        calls: list[tuple[int, str | None]] = []

        async def send_chunks(
            target: resume_command.ChannelLike,
            text: str,
            *,
            context: str = "send_chunks",
        ) -> int:
            _ = target, context
            sent.append(text)
            return 1

        async def recover(channel_id: int, ref: str | None) -> str:
            calls.append((channel_id, ref))
            return "recovered"

        handled = await resume_command.handle_prefix_resume_command(
            "resume",
            "",
            FakeMessage(),
            deps=resume_command.PrefixResumeCommandDeps(
                send_chunks=send_chunks,
                recover_resident_thread_for_request=recover,
                log_line=lambda _line: None,
            ),
        )

        self.assertTrue(handled)
        self.assertEqual(calls, [(123, None)])
        self.assertEqual(sent, ["recovered"])

    async def test_resume_failure_surfaces_actual_error(self) -> None:
        sent: list[str] = []
        logs: list[str] = []

        async def send_chunks(
            target: resume_command.ChannelLike,
            text: str,
            *,
            context: str = "send_chunks",
        ) -> int:
            _ = target, context
            sent.append(text)
            return 1

        async def recover(channel_id: int, ref: str | None) -> str:
            _ = channel_id, ref
            raise TimeoutError("Timed out waiting for app-server response to thread/resume.")

        handled = await resume_command.handle_prefix_resume_command(
            "resume",
            "taxlab:2",
            FakeMessage(),
            deps=resume_command.PrefixResumeCommandDeps(
                send_chunks=send_chunks,
                recover_resident_thread_for_request=recover,
                log_line=logs.append,
            ),
        )

        self.assertTrue(handled)
        self.assertIn("Timed out waiting for app-server response to thread/resume.", sent[0])
        self.assertIn("resident_thread_resume_failed", logs[0])


if __name__ == "__main__":
    _ = unittest.main()
