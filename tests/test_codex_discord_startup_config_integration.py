from __future__ import annotations

import io
import os
import sqlite3
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from types import TracebackType
from typing import final

import codex_discord_bot as bot


@final
class EnvPatch:
    def __init__(self, key: str, value: str) -> None:
        self.key = key
        self.value = value
        self.original: str | None = None

    def __enter__(self) -> None:
        self.original = os.environ.get(self.key)
        os.environ[self.key] = self.value

    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        tb: TracebackType | None,
    ) -> None:
        _ = exc_type, exc, tb
        if self.original is None:
            _ = os.environ.pop(self.key, None)
        else:
            os.environ[self.key] = self.original


@final
class DiscordStartupConfigIntegrationTests(unittest.TestCase):
    def test_startup_probe_targets_include_allowed_and_mirror_channels(self) -> None:
        old_db_path = bot.MIRROR_DB_PATH
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            bot.MIRROR_DB_PATH = Path(temp_dir) / "mirror.sqlite"
            try:
                bot.init_mirror_db()
                with sqlite3.connect(bot.MIRROR_DB_PATH) as conn:
                    _ = conn.execute(
                        "INSERT INTO mirror_projects (project_key, project_name, "
                        + "discord_channel_id, updated_at) VALUES (?, ?, ?, ?)",
                        ("c:/taxlab", "taxlab", 333, 20.0),
                    )
                    _ = conn.execute(
                        "INSERT INTO mirror_threads (codex_thread_id, project_key, thread_title, "
                        + "discord_channel_id, discord_thread_id, updated_at) "
                        + "VALUES (?, ?, ?, ?, ?, ?)",
                        ("thread-1", "c:/taxlab", "title", 333, 444, 30.0),
                    )
                targets = bot.get_startup_probe_targets({111, 222}, 111)
            finally:
                bot.MIRROR_DB_PATH = old_db_path

        self.assertEqual(
            targets,
            [
                ("startup", 111),
                ("allowed", 222),
                ("mirror_project", 333),
                ("mirror_thread", 444),
            ],
        )

    def test_main_requires_allowed_channels_unless_explicit_all(self) -> None:
        old_env = {key: os.environ.get(key) for key in os.environ if key.startswith("DISCORD_")}
        original_env_path = bot.ENV_PATH
        original_argv = sys.argv[:]
        try:
            for key in list(os.environ):
                if key.startswith("DISCORD_"):
                    _ = os.environ.pop(key, None)
            os.environ["DISCORD_BOT_TOKEN"] = "fake-token"
            sys.argv = ["codex_discord_bot.py"]
            stdout = io.StringIO()
            with tempfile.TemporaryDirectory() as temp_dir:
                bot.ENV_PATH = Path(temp_dir) / "missing.env"
                log_path = Path(temp_dir) / "discord-smoke.log"
                with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)), redirect_stdout(stdout):
                    exit_code = bot.main()
                log_text = log_path.read_text(encoding="utf-8")
        finally:
            bot.ENV_PATH = original_env_path
            sys.argv = original_argv
            for key in list(os.environ):
                if key.startswith("DISCORD_"):
                    _ = os.environ.pop(key, None)
            for key, value in old_env.items():
                if value is not None:
                    os.environ[key] = value

        self.assertEqual(exit_code, 1)
        self.assertIn("DISCORD_ALLOWED_CHANNEL_IDS", stdout.getvalue())
        self.assertIn("main_config_error reason=missing_allowed_channels", log_text)


if __name__ == "__main__":
    _ = unittest.main()
