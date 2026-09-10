from __future__ import annotations

# pyright: reportAssignmentType=false, reportAttributeAccessIssue=false, reportUnknownMemberType=false, reportUnknownVariableType=false
from dataclasses import dataclass
from pathlib import Path
import tempfile
from typing import override
import unittest

import codex_discord_bot as bot
import codex_desktop_bridge as bridge
from codex_thread_models import ThreadInfo

from tests.test_codex_discord_bot import EnvPatch, FakeBot


@dataclass(frozen=True, slots=True)
class MirrorThread:
    id: int


class RouteResolutionError(RuntimeError):
    @override
    def __str__(self) -> str:
        return "bad route"


class RouteDependencyTypeError(TypeError):
    @override
    def __str__(self) -> str:
        return "bad route dependency"


class DiscordNewThreadIntegrationTests(unittest.IsolatedAsyncioTestCase):
    async def test_new_thread_preferred_channel_type_error_is_not_mirror_failure(self) -> None:
        original_resolve_cwd = bot.resolve_discord_new_thread_cwd
        original_resolve_project_channel = bot.resolve_discord_new_thread_project_channel_id
        original_run_bridge_command = bot.run_bridge_command
        original_mirror_single = bot.mirror_single_codex_thread
        original_choose_thread = bridge.choose_thread
        mirror_calls: list[tuple[str, int | None]] = []

        def fake_resolve_cwd(discord_channel_id: int | None) -> str:
            _ = discord_channel_id
            return r"C:\taxlab"

        def fail_resolve_project_channel(discord_channel_id: int | None, project_key: str | None) -> int:
            _ = (discord_channel_id, project_key)
            raise RouteDependencyTypeError()

        def fake_choose_thread(thread_id: str | None = None, ref: str | None = None) -> ThreadInfo:
            _ = ref
            return ThreadInfo(
                id=str(thread_id or ""),
                title="new",
                cwd=r"C:\taxlab",
                updated_at=1,
                rollout_path="",
                model="",
                reasoning_effort="",
                tokens_used=0,
            )

        def fake_run_bridge_command(argv: list[str]) -> tuple[int, str]:
            _ = argv
            return 0, "target_thread: thread-new\ncwd: C:\\taxlab"

        async def fake_mirror_single_codex_thread(
            fake_bot: object,
            thread_id: str,
            *,
            preferred_project_channel_id: int | None = None,
        ) -> MirrorThread:
            _ = fake_bot
            mirror_calls.append((thread_id, preferred_project_channel_id))
            return MirrorThread(999)

        try:
            bot.resolve_discord_new_thread_cwd = fake_resolve_cwd
            bot.resolve_discord_new_thread_project_channel_id = fail_resolve_project_channel
            bridge.choose_thread = fake_choose_thread
            bot.run_bridge_command = fake_run_bridge_command
            bot.mirror_single_codex_thread = fake_mirror_single_codex_thread

            with tempfile.TemporaryDirectory() as temp_dir:
                log_path = Path(temp_dir) / "discord-smoke.log"
                with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                    with self.assertRaisesRegex(TypeError, "bad route dependency"):
                        _ = await bot.run_discord_new_thread(
                            FakeBot(),
                            222,
                            "start here",
                        )
                log_text = log_path.read_text(encoding="utf-8") if log_path.exists() else ""

            self.assertEqual(mirror_calls, [])
            self.assertNotIn("new_thread_mirror_failed", log_text)
        finally:
            bot.resolve_discord_new_thread_cwd = original_resolve_cwd
            bot.resolve_discord_new_thread_project_channel_id = original_resolve_project_channel
            bot.run_bridge_command = original_run_bridge_command
            bot.mirror_single_codex_thread = original_mirror_single
            bridge.choose_thread = original_choose_thread

    async def test_new_thread_preferred_channel_resolve_failure_does_not_mirror(self) -> None:
        original_resolve_cwd = bot.resolve_discord_new_thread_cwd
        original_resolve_project_channel = bot.resolve_discord_new_thread_project_channel_id
        original_run_bridge_command = bot.run_bridge_command
        original_mirror_single = bot.mirror_single_codex_thread
        original_choose_thread = bridge.choose_thread
        mirror_calls: list[tuple[str, int | None]] = []

        def fake_resolve_cwd(discord_channel_id: int | None) -> str:
            _ = discord_channel_id
            return r"C:\taxlab"

        def fail_resolve_project_channel(discord_channel_id: int | None, project_key: str | None) -> int:
            _ = (discord_channel_id, project_key)
            raise RouteResolutionError()

        def fake_choose_thread(thread_id: str | None = None, ref: str | None = None) -> ThreadInfo:
            _ = ref
            return ThreadInfo(
                id=str(thread_id or ""),
                title="new",
                cwd=r"C:\taxlab",
                updated_at=1,
                rollout_path="",
                model="",
                reasoning_effort="",
                tokens_used=0,
            )

        def fake_run_bridge_command(argv: list[str]) -> tuple[int, str]:
            _ = argv
            return 0, "target_thread: thread-new\ncwd: C:\\taxlab"

        async def fake_mirror_single_codex_thread(
            fake_bot: object,
            thread_id: str,
            *,
            preferred_project_channel_id: int | None = None,
        ) -> MirrorThread:
            _ = fake_bot
            mirror_calls.append((thread_id, preferred_project_channel_id))
            return MirrorThread(999)

        try:
            bot.resolve_discord_new_thread_cwd = fake_resolve_cwd
            bot.resolve_discord_new_thread_project_channel_id = fail_resolve_project_channel
            bridge.choose_thread = fake_choose_thread
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

            self.assertEqual(exit_code, 0)
            self.assertEqual(mirror_calls, [])
            self.assertIn("Mirror update failed: RouteResolutionError: bad route", output)
            self.assertIn("new_thread_mirror_failed", log_text)
            self.assertNotIn("new_thread_preferred_channel_resolve_failed", log_text)
        finally:
            bot.resolve_discord_new_thread_cwd = original_resolve_cwd
            bot.resolve_discord_new_thread_project_channel_id = original_resolve_project_channel
            bot.run_bridge_command = original_run_bridge_command
            bot.mirror_single_codex_thread = original_mirror_single
            bridge.choose_thread = original_choose_thread

    async def test_new_thread_flow_uses_resolved_cwd_and_mirrors(self) -> None:
        original_resolve_cwd = bot.resolve_discord_new_thread_cwd
        original_resolve_project_channel = bot.resolve_discord_new_thread_project_channel_id
        original_run_bridge_command = bot.run_bridge_command
        original_mirror_single = bot.mirror_single_codex_thread
        original_choose_thread = bridge.choose_thread
        argv_seen: list[str] = []
        mirror_calls: list[tuple[str, int | None]] = []

        def fake_resolve_cwd(discord_channel_id: int | None) -> str:
            _ = discord_channel_id
            return r"C:\taxlab"

        def fake_resolve_project_channel(discord_channel_id: int | None, project_key: str | None) -> int:
            _ = (discord_channel_id, project_key)
            return 777

        def fake_choose_thread(thread_id: str | None = None, ref: str | None = None) -> ThreadInfo:
            _ = ref
            return ThreadInfo(
                id=str(thread_id or ""),
                title="new",
                cwd=r"C:\taxlab",
                updated_at=1,
                rollout_path="",
                model="",
                reasoning_effort="",
                tokens_used=0,
            )

        def fake_run_bridge_command(argv: list[str]) -> tuple[int, str]:
            argv_seen.extend(argv)
            return 0, "target_thread: thread-new\ncwd: C:\\taxlab"

        async def fake_mirror_single_codex_thread(
            fake_bot: object,
            thread_id: str,
            *,
            preferred_project_channel_id: int | None = None,
        ) -> MirrorThread:
            _ = fake_bot
            mirror_calls.append((thread_id, preferred_project_channel_id))
            return MirrorThread(999)

        try:
            bot.resolve_discord_new_thread_cwd = fake_resolve_cwd
            bot.resolve_discord_new_thread_project_channel_id = fake_resolve_project_channel
            bridge.choose_thread = fake_choose_thread
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

            self.assertEqual(exit_code, 0)
            self.assertEqual(argv_seen, ["new", "--cwd", r"C:\taxlab", "start here"])
            self.assertEqual(mirror_calls, [("thread-new", 777)])
            self.assertIn("target_thread: thread-new", output)
            self.assertIn("Mirrored Discord thread: <#999>", output)
        finally:
            bot.resolve_discord_new_thread_cwd = original_resolve_cwd
            bot.resolve_discord_new_thread_project_channel_id = original_resolve_project_channel
            bot.run_bridge_command = original_run_bridge_command
            bot.mirror_single_codex_thread = original_mirror_single
            bridge.choose_thread = original_choose_thread


if __name__ == "__main__":
    _ = unittest.main()
