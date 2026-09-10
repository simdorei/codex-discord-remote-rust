from __future__ import annotations

import os
import tempfile
import unittest
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Protocol, TypeAlias, cast, final
from unittest import mock

import codex_app_server_transport as app_server_transport
import codex_discord_bot as bot


class AppServerUnavailableError(RuntimeError):
    pass


class BadAppServerDependencyError(TypeError):
    pass


class BadSlashSyncDependencyError(TypeError):
    pass


class GuildLike(Protocol):
    pass


@dataclass(frozen=True, slots=True)
class FakeCommand:
    name: str


SlashSyncError: TypeAlias = RuntimeError | BadSlashSyncDependencyError


@final
class FakeTree:
    def __init__(self, sync_error: SlashSyncError | None = None) -> None:
        self.sync_error = sync_error

    async def sync(self, guild: GuildLike | None = None) -> list[FakeCommand]:
        _ = guild
        if self.sync_error is not None:
            raise self.sync_error
        return [FakeCommand(name="ask")]

    def copy_global_to(self, *, guild: GuildLike) -> None:
        _ = guild


@final
class FakeClient:
    def __init__(self, tree: FakeTree, *, command_status: str = "-") -> None:
        self.guild_id: int | None = None
        self.tree = tree
        self._slash_sync_last_at = "-"
        self._slash_sync_status = "-"
        self._slash_sync_commands = command_status

    def slash_sync_status(self) -> str:
        return self._slash_sync_status

    def slash_sync_commands(self) -> str:
        return self._slash_sync_commands


SetupHookFunc: TypeAlias = Callable[[FakeClient], Awaitable[None]]


def register_commands(client: FakeClient) -> None:
    _ = client


async def run_setup_hook(client: FakeClient) -> None:
    setup_hook = cast(SetupHookFunc, bot.CodexDiscordBot.setup_hook)
    await setup_hook(client)


def read_log_text(log_path: Path) -> str:
    if not log_path.exists():
        return ""
    return log_path.read_text(encoding="utf-8")


@final
class DiscordSetupHookIntegrationTests(unittest.IsolatedAsyncioTestCase):
    async def test_setup_hook_app_server_runtime_failure_logs_and_continues(self) -> None:
        client = FakeClient(tree=FakeTree())

        with tempfile.TemporaryDirectory() as temp_dir:
            log_path = Path(temp_dir) / "discord-smoke.log"
            with (
                mock.patch.dict(os.environ, {"CODEX_DISCORD_LOG_PATH": str(log_path)}),
                mock.patch.object(bot, "app_server_transport_enabled", return_value=True),
                mock.patch.object(
                    app_server_transport.DEFAULT_CLIENT,
                    "start",
                    side_effect=AppServerUnavailableError("app server unavailable"),
                ),
                mock.patch.object(bot, "register_commands", register_commands),
            ):
                await run_setup_hook(client)
            log_text = read_log_text(log_path)

        self.assertEqual(client.slash_sync_status(), "ok")
        self.assertEqual(client.slash_sync_commands(), "ask")
        self.assertIn("setup_hook_app_server_transport_failed error=app server unavailable", log_text)
        self.assertIn("setup_hook_synced commands=ask", log_text)

    async def test_setup_hook_app_server_type_error_is_not_startup_failure(self) -> None:
        client = FakeClient(tree=FakeTree())

        with tempfile.TemporaryDirectory() as temp_dir:
            log_path = Path(temp_dir) / "discord-smoke.log"
            with (
                mock.patch.dict(os.environ, {"CODEX_DISCORD_LOG_PATH": str(log_path)}),
                mock.patch.object(bot, "app_server_transport_enabled", return_value=True),
                mock.patch.object(
                    app_server_transport.DEFAULT_CLIENT,
                    "start",
                    side_effect=BadAppServerDependencyError("bad app server dependency"),
                ),
                mock.patch.object(bot, "register_commands", register_commands),
            ):
                with self.assertRaisesRegex(TypeError, "bad app server dependency"):
                    await run_setup_hook(client)
            log_text = read_log_text(log_path)

        self.assertNotIn("setup_hook_app_server_transport_failed", log_text)

    async def test_setup_hook_slash_sync_runtime_failure_marks_skipped(self) -> None:
        client = FakeClient(tree=FakeTree(RuntimeError("sync unavailable")), command_status="old")

        with tempfile.TemporaryDirectory() as temp_dir:
            log_path = Path(temp_dir) / "discord-smoke.log"
            with (
                mock.patch.dict(os.environ, {"CODEX_DISCORD_LOG_PATH": str(log_path)}),
                mock.patch.object(bot, "app_server_transport_enabled", return_value=False),
                mock.patch.object(bot, "register_commands", register_commands),
            ):
                await run_setup_hook(client)
            log_text = read_log_text(log_path)

        self.assertEqual(client.slash_sync_status(), "skipped:RuntimeError")
        self.assertEqual(client.slash_sync_commands(), "-")
        self.assertIn("setup_hook_sync_skipped error=sync unavailable", log_text)
        self.assertIn("setup_hook_done", log_text)

    async def test_setup_hook_slash_sync_type_error_is_not_sync_skipped(self) -> None:
        client = FakeClient(
            tree=FakeTree(BadSlashSyncDependencyError("bad slash sync dependency")),
            command_status="old",
        )

        with tempfile.TemporaryDirectory() as temp_dir:
            log_path = Path(temp_dir) / "discord-smoke.log"
            with (
                mock.patch.dict(os.environ, {"CODEX_DISCORD_LOG_PATH": str(log_path)}),
                mock.patch.object(bot, "app_server_transport_enabled", return_value=False),
                mock.patch.object(bot, "register_commands", register_commands),
            ):
                with self.assertRaisesRegex(TypeError, "bad slash sync dependency"):
                    await run_setup_hook(client)
            log_text = read_log_text(log_path)

        self.assertNotIn("setup_hook_sync_skipped", log_text)
        self.assertEqual(client.slash_sync_status(), "-")
        self.assertEqual(client.slash_sync_commands(), "old")


if __name__ == "__main__":
    _ = unittest.main()
