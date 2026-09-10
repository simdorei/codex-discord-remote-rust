from __future__ import annotations

from collections.abc import Awaitable
from pathlib import Path
from typing import Protocol, cast
import tempfile
import unittest

import codex_discord_bot as bot
from codex_discord_text import DISCORD_MAX_LEN

from tests.test_codex_discord_bot import EnvPatch, FakeTarget


class RunBridgeAndSend(Protocol):
    def __call__(
        self,
        target: FakeTarget,
        argv: list[str],
        title: str,
        failure_title: str | None = None,
    ) -> Awaitable[tuple[int, str]]:
        ...


def _run_bridge_and_send() -> RunBridgeAndSend:
    return cast(RunBridgeAndSend, bot.run_bridge_and_send)


class DiscordBridgeSendIntegrationTests(unittest.IsolatedAsyncioTestCase):
    async def test_run_bridge_and_send_logs_and_sends(self) -> None:
        original_run_bridge_command = bot.run_bridge_command
        try:
            def fake_run_bridge_command(argv: list[str]) -> tuple[int, str]:
                _ = argv
                return 0, "ok"

            bot.run_bridge_command = fake_run_bridge_command
            with tempfile.TemporaryDirectory() as temp_dir:
                log_path = Path(temp_dir) / "discord-smoke.log"
                target = FakeTarget()
                with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                    exit_code, output = await _run_bridge_and_send()(
                        target,
                        ["status"],
                        "Status",
                    )

                self.assertEqual(exit_code, 0)
                self.assertEqual(output, "ok")
                self.assertEqual(target.messages, [("Status\n\nok", None)])
                log_text = log_path.read_text(encoding="utf-8")
                self.assertIn("bridge_command_done title='Status' exit=0", log_text)
                self.assertIn("discord_delivery_start", log_text)
                self.assertIn("context=bridge_command:Status", log_text)
                self.assertIn("bridge_command_sent title='Status' exit=0", log_text)
        finally:
            bot.run_bridge_command = original_run_bridge_command

    async def test_run_bridge_and_send_uses_marked_delivery_chunks_for_long_output(self) -> None:
        original_run_bridge_command = bot.run_bridge_command
        original_chunk_markers = bot.DISCORD_CHUNK_MARKERS_ENABLED
        try:
            def fake_run_bridge_command(argv: list[str]) -> tuple[int, str]:
                _ = argv
                return 0, "x" * 4100

            bot.run_bridge_command = fake_run_bridge_command
            bot.DISCORD_CHUNK_MARKERS_ENABLED = True
            target = FakeTarget()

            _ = await _run_bridge_and_send()(
                target,
                ["open", "thread-1"],
                "Open",
            )

            sent = [content for content, _view in target.messages]
            self.assertGreater(len(sent), 1)
            self.assertTrue(sent[0].startswith("[1/"))
            self.assertTrue(sent[-1].startswith(f"[{len(sent)}/{len(sent)}]"))
            self.assertTrue(all(len(content) <= DISCORD_MAX_LEN for content in sent))
        finally:
            bot.run_bridge_command = original_run_bridge_command
            bot.DISCORD_CHUNK_MARKERS_ENABLED = original_chunk_markers


if __name__ == "__main__":
    _ = unittest.main()
