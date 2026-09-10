from __future__ import annotations

import os
import tempfile
import time
import unittest
from pathlib import Path
from types import TracebackType
from typing import final
from unittest.mock import patch

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
class DiscordRuntimeLockIntegrationTests(unittest.TestCase):
    def test_runtime_instance_lock_blocks_second_holder(self) -> None:
        if os.name != "nt":
            self.skipTest("Windows runtime mutex is only available on Windows")
        mutex_name = f"Local\\CodexDiscordBotTest_{os.getpid()}_{time.time_ns()}"
        old_runtime_lock_path = bot.RUNTIME_LOCK_PATH
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            bot.RUNTIME_LOCK_PATH = Path(temp_dir) / "runtime.lock"
            try:
                with bot.acquire_runtime_instance_lock(mutex_name) as first:
                    self.assertTrue(first)
                    with bot.acquire_runtime_instance_lock(mutex_name) as second:
                        self.assertFalse(second)
                with bot.acquire_runtime_instance_lock(mutex_name) as third:
                    self.assertTrue(third)
                self.assertFalse(bot.RUNTIME_LOCK_PATH.exists())
            finally:
                bot.RUNTIME_LOCK_PATH = old_runtime_lock_path

    def test_runtime_lock_remove_failure_is_logged(self) -> None:
        old_runtime_lock_path = bot.RUNTIME_LOCK_PATH
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            lock_path = Path(temp_dir) / "runtime.lock"
            log_path = Path(temp_dir) / "discord-smoke.log"
            _ = lock_path.write_text(str(os.getpid()), encoding="ascii")
            bot.RUNTIME_LOCK_PATH = lock_path
            try:
                with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                    with patch.object(Path, "unlink", side_effect=OSError("locked")):
                        bot.remove_runtime_lock_for_current_process(reason="test_failure")
                log_text = log_path.read_text(encoding="utf-8") if log_path.exists() else ""
            finally:
                bot.RUNTIME_LOCK_PATH = old_runtime_lock_path

            self.assertIn("runtime_lock_remove_failed", log_text)
            self.assertIn("reason=test_failure", log_text)
            self.assertIn("error_type=OSError error=locked", log_text)


if __name__ == "__main__":
    _ = unittest.main()
