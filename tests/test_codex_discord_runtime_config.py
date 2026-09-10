from __future__ import annotations

import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import codex_discord_runtime_config as runtime_config


class RuntimeConfigTests(unittest.TestCase):
    def test_get_required_env_returns_stripped_value(self) -> None:
        with patch.dict(os.environ, {"DISCORD_BOT_TOKEN": " token-value \n"}, clear=True):
            self.assertEqual(
                runtime_config.get_required_env("DISCORD_BOT_TOKEN"),
                "token-value",
            )

    def test_get_required_env_raises_for_missing_value(self) -> None:
        with patch.dict(os.environ, {}, clear=True):
            with self.assertRaisesRegex(runtime_config.MissingRequiredEnvError, "DISCORD_BOT_TOKEN") as caught:
                _ = runtime_config.get_required_env("DISCORD_BOT_TOKEN")
            self.assertEqual(caught.exception.name, "DISCORD_BOT_TOKEN")

    def test_get_required_env_raises_for_blank_value(self) -> None:
        with patch.dict(os.environ, {"DISCORD_BOT_TOKEN": " \t"}, clear=True):
            with self.assertRaisesRegex(
                RuntimeError,
                "Missing required environment variable: DISCORD_BOT_TOKEN",
            ):
                _ = runtime_config.get_required_env("DISCORD_BOT_TOKEN")

    def test_discord_qa_commands_enabled_defaults_to_false(self) -> None:
        with patch.dict(os.environ, {}, clear=True):
            self.assertFalse(runtime_config.discord_qa_commands_enabled())

    def test_discord_qa_commands_enabled_accepts_truthy_env(self) -> None:
        with patch.dict(os.environ, {"DISCORD_ENABLE_QA_COMMANDS": "yes"}, clear=True):
            self.assertTrue(runtime_config.discord_qa_commands_enabled())

    def test_discord_qa_commands_enabled_accepts_falsey_env(self) -> None:
        with patch.dict(os.environ, {"DISCORD_ENABLE_QA_COMMANDS": "off"}, clear=True):
            self.assertFalse(runtime_config.discord_qa_commands_enabled())

    def test_discord_host_commands_enabled_defaults_to_false(self) -> None:
        with patch.dict(os.environ, {}, clear=True):
            self.assertFalse(runtime_config.discord_host_commands_enabled())

    def test_discord_host_commands_enabled_accepts_truthy_env(self) -> None:
        with patch.dict(os.environ, {"DISCORD_ENABLE_HOST_COMMANDS": "yes"}, clear=True):
            self.assertTrue(runtime_config.discord_host_commands_enabled())

    def test_discord_host_commands_enabled_accepts_falsey_env(self) -> None:
        with patch.dict(os.environ, {"DISCORD_ENABLE_HOST_COMMANDS": "off"}, clear=True):
            self.assertFalse(runtime_config.discord_host_commands_enabled())

    def test_discord_stream_commentary_enabled_defaults_to_true(self) -> None:
        with patch.dict(os.environ, {}, clear=True):
            self.assertTrue(runtime_config.discord_stream_commentary_enabled())

    def test_discord_stream_commentary_enabled_accepts_falsey_env(self) -> None:
        with patch.dict(os.environ, {"DISCORD_STREAM_COMMENTARY": "0"}, clear=True):
            self.assertFalse(runtime_config.discord_stream_commentary_enabled())

    def test_discord_stream_commentary_enabled_accepts_truthy_env(self) -> None:
        with patch.dict(os.environ, {"DISCORD_STREAM_COMMENTARY": "yes"}, clear=True):
            self.assertTrue(runtime_config.discord_stream_commentary_enabled())

    def test_discord_startup_notify_enabled_defaults_to_false(self) -> None:
        with patch.dict(os.environ, {}, clear=True):
            self.assertFalse(runtime_config.discord_startup_notify_enabled())

    def test_discord_startup_notify_enabled_accepts_truthy_env(self) -> None:
        with patch.dict(os.environ, {"DISCORD_STARTUP_NOTIFY": "yes"}, clear=True):
            self.assertTrue(runtime_config.discord_startup_notify_enabled())

    def test_discord_startup_notify_enabled_accepts_falsey_env(self) -> None:
        with patch.dict(os.environ, {"DISCORD_STARTUP_NOTIFY": "0"}, clear=True):
            self.assertFalse(runtime_config.discord_startup_notify_enabled())

    def test_load_local_env_reads_values_without_overwriting_existing_env(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            env_path = Path(temp_dir) / ".env"
            _ = env_path.write_text(
                "\n".join(
                    [
                        "# ignored",
                        "TOKEN = from-file",
                        'DOUBLE_QUOTED = "quoted"',
                        "SINGLE_QUOTED = 'single'",
                        "NO_EQUALS",
                        "EXISTING = file-value",
                    ]
                ),
                encoding="utf-8",
            )
            with patch.dict(os.environ, {"EXISTING": "process-value"}, clear=True):
                runtime_config.load_local_env(env_path)
                self.assertEqual(os.environ["TOKEN"], "from-file")
                self.assertEqual(os.environ["DOUBLE_QUOTED"], "quoted")
                self.assertEqual(os.environ["SINGLE_QUOTED"], "single")
                self.assertEqual(os.environ["EXISTING"], "process-value")
                self.assertNotIn("NO_EQUALS", os.environ)

    def test_load_local_env_ignores_missing_file(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            with patch.dict(os.environ, {}, clear=True):
                runtime_config.load_local_env(Path(temp_dir) / "missing.env")
                self.assertEqual(os.environ, {})

    def test_get_ask_busy_retry_attempts_defaults_to_zero(self) -> None:
        with patch.dict(os.environ, {}, clear=True):
            self.assertEqual(runtime_config.get_ask_busy_retry_attempts(default=0.0), 0)

    def test_get_ask_busy_retry_attempts_clamps_to_range(self) -> None:
        with patch.dict(os.environ, {"DISCORD_ASK_BUSY_RETRY_ATTEMPTS": "-1"}, clear=True):
            self.assertEqual(runtime_config.get_ask_busy_retry_attempts(default=2.0), 0)
        with patch.dict(os.environ, {"DISCORD_ASK_BUSY_RETRY_ATTEMPTS": "99"}, clear=True):
            self.assertEqual(runtime_config.get_ask_busy_retry_attempts(default=2.0), 10)

    def test_get_ask_busy_retry_attempts_converts_float_to_int(self) -> None:
        with patch.dict(os.environ, {"DISCORD_ASK_BUSY_RETRY_ATTEMPTS": "2.9"}, clear=True):
            self.assertEqual(runtime_config.get_ask_busy_retry_attempts(default=0.0), 2)

    def test_get_ask_busy_retry_delay_seconds_defaults_to_default(self) -> None:
        with patch.dict(os.environ, {}, clear=True):
            self.assertEqual(runtime_config.get_ask_busy_retry_delay_seconds(default=8.0), 8.0)

    def test_get_ask_busy_retry_delay_seconds_clamps_to_range(self) -> None:
        with patch.dict(os.environ, {"DISCORD_ASK_BUSY_RETRY_DELAY_SECONDS": "0"}, clear=True):
            self.assertEqual(runtime_config.get_ask_busy_retry_delay_seconds(default=8.0), 1.0)
        with patch.dict(os.environ, {"DISCORD_ASK_BUSY_RETRY_DELAY_SECONDS": "99"}, clear=True):
            self.assertEqual(runtime_config.get_ask_busy_retry_delay_seconds(default=8.0), 60.0)

    def test_get_ask_busy_retry_delay_seconds_uses_default_for_invalid_env(self) -> None:
        with patch.dict(os.environ, {"DISCORD_ASK_BUSY_RETRY_DELAY_SECONDS": "invalid"}, clear=True):
            self.assertEqual(runtime_config.get_ask_busy_retry_delay_seconds(default=8.0), 8.0)

    def test_get_steering_delivery_confirm_timeout_defaults_to_default(self) -> None:
        with patch.dict(os.environ, {}, clear=True):
            self.assertEqual(runtime_config.get_steering_delivery_confirm_timeout(default=25.0), 25.0)

    def test_get_steering_delivery_confirm_timeout_clamps_to_range(self) -> None:
        with patch.dict(
            os.environ,
            {"DISCORD_STEERING_DELIVERY_CONFIRM_TIMEOUT_SECONDS": "2"},
            clear=True,
        ):
            self.assertEqual(runtime_config.get_steering_delivery_confirm_timeout(default=25.0), 3.0)
        with patch.dict(
            os.environ,
            {"DISCORD_STEERING_DELIVERY_CONFIRM_TIMEOUT_SECONDS": "999"},
            clear=True,
        ):
            self.assertEqual(runtime_config.get_steering_delivery_confirm_timeout(default=25.0), 120.0)

    def test_get_steering_delivery_confirm_timeout_uses_default_for_invalid_env(self) -> None:
        with patch.dict(
            os.environ,
            {"DISCORD_STEERING_DELIVERY_CONFIRM_TIMEOUT_SECONDS": "invalid"},
            clear=True,
        ):
            self.assertEqual(runtime_config.get_steering_delivery_confirm_timeout(default=25.0), 25.0)

    def test_get_steering_pending_watch_timeout_defaults_to_default(self) -> None:
        with patch.dict(os.environ, {}, clear=True):
            self.assertEqual(runtime_config.get_steering_pending_watch_timeout(default=600.0), 600.0)

    def test_get_steering_pending_watch_timeout_clamps_to_range(self) -> None:
        with patch.dict(
            os.environ,
            {"DISCORD_STEERING_PENDING_WATCH_TIMEOUT_SECONDS": "2"},
            clear=True,
        ):
            self.assertEqual(runtime_config.get_steering_pending_watch_timeout(default=600.0), 10.0)
        with patch.dict(
            os.environ,
            {"DISCORD_STEERING_PENDING_WATCH_TIMEOUT_SECONDS": "999"},
            clear=True,
        ):
            self.assertEqual(runtime_config.get_steering_pending_watch_timeout(default=600.0), 600.0)

    def test_get_steering_pending_watch_timeout_uses_default_for_invalid_env(self) -> None:
        with patch.dict(
            os.environ,
            {"DISCORD_STEERING_PENDING_WATCH_TIMEOUT_SECONDS": "invalid"},
            clear=True,
        ):
            self.assertEqual(runtime_config.get_steering_pending_watch_timeout(default=600.0), 600.0)

    def test_get_stale_busy_steer_block_seconds_defaults_to_default(self) -> None:
        with patch.dict(os.environ, {}, clear=True):
            self.assertEqual(runtime_config.get_stale_busy_steer_block_seconds(default=600.0), 600.0)

    def test_get_stale_busy_steer_block_seconds_clamps_to_range(self) -> None:
        with patch.dict(
            os.environ,
            {"DISCORD_STALE_BUSY_STEER_BLOCK_SECONDS": "2"},
            clear=True,
        ):
            self.assertEqual(runtime_config.get_stale_busy_steer_block_seconds(default=600.0), 60.0)
        with patch.dict(
            os.environ,
            {"DISCORD_STALE_BUSY_STEER_BLOCK_SECONDS": "9999"},
            clear=True,
        ):
            self.assertEqual(runtime_config.get_stale_busy_steer_block_seconds(default=600.0), 3600.0)

    def test_get_stale_busy_steer_block_seconds_uses_default_for_invalid_env(self) -> None:
        with patch.dict(
            os.environ,
            {"DISCORD_STALE_BUSY_STEER_BLOCK_SECONDS": "invalid"},
            clear=True,
        ):
            self.assertEqual(runtime_config.get_stale_busy_steer_block_seconds(default=600.0), 600.0)

    def test_get_history_poll_seconds_defaults_to_default(self) -> None:
        with patch.dict(os.environ, {}, clear=True):
            self.assertEqual(runtime_config.get_history_poll_seconds(default=15.0), 15.0)

    def test_get_history_poll_seconds_clamps_to_range(self) -> None:
        with patch.dict(os.environ, {"DISCORD_HISTORY_POLL_SECONDS": "-1"}, clear=True):
            self.assertEqual(runtime_config.get_history_poll_seconds(default=15.0), 0.0)
        with patch.dict(os.environ, {"DISCORD_HISTORY_POLL_SECONDS": "999"}, clear=True):
            self.assertEqual(runtime_config.get_history_poll_seconds(default=15.0), 300.0)

    def test_get_history_poll_seconds_uses_default_for_invalid_env(self) -> None:
        with patch.dict(os.environ, {"DISCORD_HISTORY_POLL_SECONDS": "invalid"}, clear=True):
            self.assertEqual(runtime_config.get_history_poll_seconds(default=15.0), 15.0)
