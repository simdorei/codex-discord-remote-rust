from __future__ import annotations

from pathlib import Path
from types import SimpleNamespace
import os
import sqlite3
import tempfile
import unittest
from unittest import mock

import codex_discord_bot as bot


def _missing_mirror_check() -> str:
    return (_ for _ in ()).throw(FileNotFoundError("missing state db"))


class DiscordDoctorIntegrationTests(unittest.TestCase):
    def test_discord_doctor_message_includes_adapter_diagnostics(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            mirror_path = Path(temp_dir) / "mirror.sqlite"
            log_path = Path(temp_dir) / "discord-smoke.log"
            with (
                mock.patch.object(bot, "MIRROR_DB_PATH", mirror_path),
                mock.patch.object(bot, "build_mirror_check", lambda: "Mirror check\ncodex_threads: 0"),
            ):
                bot.init_mirror_db()
                with sqlite3.connect(bot.MIRROR_DB_PATH) as conn:
                    insert_claims_sql = (
                        "INSERT INTO persistent_component_claims (claim_key, created_at, expires_at) "
                        + "VALUES ('doctor-active', 1, 9999999999), ('doctor-stale', 1, 2)"
                    )
                    _ = conn.execute(insert_claims_sql)
                fake_bot = SimpleNamespace(
                    enable_prefix_commands=True,
                    intents=SimpleNamespace(message_content=True),
                    _enable_debug_events=True,
                    allowed_channel_ids={222},
                    allowed_user_ids=set(),
                    startup_channel_id=222,
                    history_poll_seconds=15.0,
                    _history_poll_task=SimpleNamespace(done=lambda: False),
                    _history_poll_last_at="2026-06-03T06:23:10+00:00",
                    _history_poll_primed_channels={111, 222},
                    _slash_sync_status="ok",
                    _slash_sync_last_at="2026-06-03T06:23:07+00:00",
                    _slash_sync_commands="ask,doctor,new",
                )
                _ = log_path.write_text(
                    "\n".join(
                        [
                            "[2026-06-03 13:59:57] ready user=codex#1234 guilds=1",
                            "[2026-06-03 13:59:58] socket_message_create channel=222 tracked=True source=client_channel_cache guild=1 author=3 bot=True content_len=49",
                            "[2026-06-03 13:59:59] socket_message_create channel=222 tracked=True source=client_channel_cache guild=1 author=3 bot=True content_len=49",
                            "[2026-06-03 14:00:00] socket_message_create channel=222 tracked=True source=client_channel_cache guild=1 author=2 bot=False content_len=12",
                            "[2026-06-03 14:00:01] message chat=222 user=2 prefix=False runner_busy=False codex_busy=idle target_source=mirror target=thread-1 text=sensitive prompt",
                            "[2026-06-03 14:00:02] busy_choice_sent reason=late_busy_failure target=thread-1 prompt_len=16",
                            "[2026-06-03 14:00:03] slash_ask_dispatch command=ask channel=222 user=2 target_source=mirror target=thread-1 prompt_len=9",
                            "[2026-06-03 14:00:04] slash_response_sent command=doctor title='Doctor' exit=0 chunks=1",
                            "[2026-06-03 14:00:05] socket_interaction_create channel=222 guild=1 user=2 type=2 command=ask",
                            "[2026-06-03 14:00:06] interaction_received type=application_command command=ask custom_id=- channel=222 user=2",
                            "[2026-06-03 14:00:07] interaction_received type=component command=- custom_id=codex_busy:abcd:queue channel=222 user=2",
                            "[2026-06-03 14:00:08] component_interaction_unhandled_reported custom_id=codex_busy:abcd:queue channel=222",
                            "[2026-06-03 14:00:09] button_qa_done channel=222 user=2 result=ok",
                            "[2026-06-03 14:00:10] steer_now_done exit=0 target=thread-1 elapsed_sec=6.12 output_len=42",
                        ]
                    ),
                    encoding="utf-8",
                )
                with mock.patch.dict(
                    os.environ,
                    {"CODEX_DISCORD_LOG_PATH": str(log_path), "DISCORD_ENABLE_QA_COMMANDS": "1"},
                ):
                    output = bot.build_discord_doctor_message(fake_bot, 222)

        self.assertIn("Discord adapter diagnostics", output)
        self.assertIn("channel_id: 222", output)
        self.assertIn("message_content_enabled: True", output)
        self.assertIn("intent_message_content: True", output)
        self.assertIn("raw_debug_events: True", output)
        self.assertIn("qa_commands_enabled: True", output)
        self.assertIn("history_poll_seconds: 15.0", output)
        self.assertIn("history_poll_alive: True", output)
        self.assertIn("history_poll_last_at: 2026-06-03T06:23:10+00:00", output)
        self.assertIn("history_poll_primed_channels: 2", output)
        self.assertIn("slash_sync_status: ok", output)
        self.assertIn("slash_sync_last_at: 2026-06-03T06:23:07+00:00", output)
        self.assertIn("slash_sync_commands: ask,doctor,new", output)
        self.assertIn("allowed_channels: 222", output)
        self.assertIn("last_ready_at: 2026-06-03 13:59:57", output)
        self.assertIn("last_gateway_event_at: 2026-06-03 14:00:05", output)
        self.assertIn("last_raw_interaction_at: 2026-06-03 14:00:05", output)
        self.assertIn("last_interaction_at: 2026-06-03 14:00:07", output)
        self.assertIn("last_component_at: 2026-06-03 14:00:08", output)
        self.assertIn("last_user_or_control_hook_at: 2026-06-03 14:00:08", output)
        self.assertIn("last_button_qa_at: 2026-06-03 14:00:09", output)
        self.assertIn("last_button_qa_result: ok", output)
        self.assertIn("persistent_component_claims_active: 1", output)
        self.assertIn("persistent_component_claims_stale: 1", output)
        self.assertIn("last_steering_button_at: 2026-06-03 14:00:10", output)
        self.assertIn("last_steering_button_exit: 0", output)
        self.assertIn("last_steering_button_elapsed_sec: 6.12", output)
        self.assertIn("Mirror check", output)
        self.assertIn("Expected live log sequence:", output)
        self.assertIn("Recent user/control hook events:", output)
        self.assertIn("Recent hook events:", output)
        self.assertIn("message_routed channel=222", output)
        self.assertIn("busy_choice_event reason=late_busy_failure", output)
        self.assertIn("slash_ask_dispatch channel=222 command=ask", output)
        self.assertIn("slash_response_sent channel=- command=doctor exit=0", output)
        self.assertIn("raw_interaction channel=222 type=2 command=ask", output)
        self.assertIn("interaction_received channel=222 type=component command=-", output)
        self.assertIn("component_event channel=222 custom_id=codex_busy:abcd:queue", output)
        user_section = output.split("Recent user/control hook events:", 1)[1].split("Recent hook events:", 1)[0]
        self.assertNotIn("bot=True", user_section)
        self.assertNotIn("sensitive prompt", output)

    def test_discord_doctor_message_surfaces_mirror_check_file_failure(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            mirror_path = Path(temp_dir) / "mirror.sqlite"
            with (
                mock.patch.object(bot, "MIRROR_DB_PATH", mirror_path),
                mock.patch.object(bot, "build_mirror_check", _missing_mirror_check),
            ):
                bot.init_mirror_db()
                output = bot.build_discord_doctor_message(SimpleNamespace(), 222)

        self.assertIn("Mirror check failed", output)
        self.assertIn("ERROR: missing state db", output)


if __name__ == "__main__":
    _ = unittest.main()
