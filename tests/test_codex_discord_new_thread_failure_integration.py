from __future__ import annotations

# pyright: reportAssignmentType=false, reportUnknownMemberType=false, reportUnknownVariableType=false
from dataclasses import dataclass
from pathlib import Path
import tempfile
import unittest

import codex_discord_bot as bot

from tests.test_codex_discord_bot import EnvPatch, FakeBot


@dataclass(frozen=True, slots=True)
class MirrorThread:
    id: int


class DiscordNewThreadFailureIntegrationTests(unittest.IsolatedAsyncioTestCase):
    async def test_new_thread_failure_does_not_mirror(self) -> None:
        original_resolve_cwd = bot.resolve_discord_new_thread_cwd
        original_run_bridge_command = bot.run_bridge_command
        original_mirror_single = bot.mirror_single_codex_thread
        argv_seen: list[str] = []
        mirror_calls: list[str] = []

        def fake_resolve_cwd(discord_channel_id: int | None) -> None:
            _ = discord_channel_id
            return None

        def fake_run_bridge_command(argv: list[str]) -> tuple[int, str]:
            argv_seen.extend(argv)
            return 1, "ERROR: cannot create thread"

        async def fake_mirror_single_codex_thread(
            fake_bot: bot.CodexDiscordBot,
            thread_id: str,
            *,
            preferred_project_channel_id: int | None = None,
        ) -> MirrorThread:
            _ = (fake_bot, preferred_project_channel_id)
            mirror_calls.append(thread_id)
            return MirrorThread(999)

        try:
            bot.resolve_discord_new_thread_cwd = fake_resolve_cwd
            bot.run_bridge_command = fake_run_bridge_command
            bot.mirror_single_codex_thread = fake_mirror_single_codex_thread

            with tempfile.TemporaryDirectory() as temp_dir:
                log_path = Path(temp_dir) / "discord-smoke.log"
                with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                    exit_code, output = await bot.run_discord_new_thread(
                        FakeBot(),
                        222,
                        "start here",
                    )
                log_text = log_path.read_text(encoding="utf-8")

            self.assertEqual(exit_code, 1)
            self.assertEqual(argv_seen, ["new", "start here"])
            self.assertEqual(mirror_calls, [])
            self.assertIn("New failed (exit 1)", output)
            self.assertNotIn("Mirrored Discord thread:", output)
            self.assertIn("new_thread_cwd channel=222 cwd=default", log_text)
            self.assertNotIn("new_thread_mirrored", log_text)
        finally:
            bot.resolve_discord_new_thread_cwd = original_resolve_cwd
            bot.run_bridge_command = original_run_bridge_command
            bot.mirror_single_codex_thread = original_mirror_single


if __name__ == "__main__":
    _ = unittest.main()
