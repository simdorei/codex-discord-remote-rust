from __future__ import annotations

from pathlib import Path
from types import SimpleNamespace
import os
import sqlite3
import tempfile
import unittest

import codex_discord_bot as bot
import codex_discord_interaction_gate as interaction_gate


class FakeAllowedBot:
    def __init__(self, *, allowed_user: bool = True, allowed_channel: bool = False) -> None:
        self.allowed_user = allowed_user
        self.allowed_channel = allowed_channel

    def is_allowed_user(self, user_id: interaction_gate.DiscordIdValue) -> bool:
        _ = user_id
        return self.allowed_user

    def is_allowed_channel(self, channel_id: interaction_gate.DiscordIdValue) -> bool:
        _ = channel_id
        return self.allowed_channel

    def is_allowed_message_channel(self, channel: interaction_gate.InteractionChannelLike) -> bool:
        _ = channel
        return False


class FakeInteractionUser:
    id: interaction_gate.DiscordIdValue = 242286902982606848


class FakeInteraction:
    def __init__(
        self,
        command_name: str = "ask",
        channel_id: interaction_gate.DiscordIdValue = 222,
    ) -> None:
        self.command = SimpleNamespace(name=command_name)
        self.channel_id: interaction_gate.DiscordIdValue = channel_id
        self.user: FakeInteractionUser | None = FakeInteractionUser()
        self.channel: interaction_gate.InteractionChannelLike | None = None


def _check_interaction_allowed(allowed_bot: FakeAllowedBot, interaction: FakeInteraction) -> bool:
    return interaction_gate.check_interaction_allowed(
        allowed_bot,
        interaction,
        log_func=bot.log_line,
        get_interaction_command_name_func=bot.get_interaction_gate_command_name,
        is_mirrored_channel_id_func=bot.is_mirrored_interaction_channel_id,
    )


class DiscordInteractionAllowedIntegrationTests(unittest.TestCase):
    _old_discord_log_path: str | None = None
    _old_mirror_db_path: Path = Path()
    _temp_dir: tempfile.TemporaryDirectory[str] | None = None

    def setUp(self) -> None:
        self._old_mirror_db_path = bot.MIRROR_DB_PATH
        self._old_discord_log_path = os.environ.get("CODEX_DISCORD_LOG_PATH")
        temp_dir = tempfile.TemporaryDirectory(ignore_cleanup_errors=True)
        self._temp_dir = temp_dir
        root = Path(temp_dir.name)
        bot.MIRROR_DB_PATH = root / "mirror.sqlite"
        os.environ["CODEX_DISCORD_LOG_PATH"] = str(root / "interaction-allowed.log")
        bot.init_mirror_db()

    def tearDown(self) -> None:
        bot.MIRROR_DB_PATH = self._old_mirror_db_path
        if self._old_discord_log_path is None:
            os.environ.pop("CODEX_DISCORD_LOG_PATH", None)
        else:
            os.environ["CODEX_DISCORD_LOG_PATH"] = self._old_discord_log_path
        if self._temp_dir is not None:
            self._temp_dir.cleanup()
            self._temp_dir = None

    def _log_text(self) -> str:
        return Path(os.environ["CODEX_DISCORD_LOG_PATH"]).read_text(encoding="utf-8")

    def test_mirrored_channel_id_authorizes_interaction_without_channel_object(self) -> None:
        with sqlite3.connect(bot.MIRROR_DB_PATH) as conn:
            conn.execute(
                """
                INSERT INTO mirror_threads (
                    codex_thread_id, project_key, thread_title,
                    discord_channel_id, discord_thread_id, updated_at
                ) VALUES (?, ?, ?, ?, ?, ?)
                """,
                ("thread-1", "project", "title", 111, 222, 1.0),
            )

        interaction = FakeInteraction(channel_id=222)

        self.assertTrue(_check_interaction_allowed(FakeAllowedBot(), interaction))

    def test_interaction_user_denial_is_logged(self) -> None:
        interaction = FakeInteraction(command_name="ask", channel_id=222)

        allowed = _check_interaction_allowed(FakeAllowedBot(allowed_user=False), interaction)

        log_text = self._log_text()
        self.assertFalse(allowed)
        self.assertIn("slash_ignored command=ask reason=user_not_allowed", log_text)
        self.assertIn("channel=222", log_text)

    def test_interaction_channel_denial_is_logged(self) -> None:
        interaction = FakeInteraction(command_name="ask", channel_id=333)

        allowed = _check_interaction_allowed(
            FakeAllowedBot(allowed_user=True, allowed_channel=False),
            interaction,
        )

        log_text = self._log_text()
        self.assertFalse(allowed)
        self.assertIn("slash_ignored command=ask reason=channel_not_allowed", log_text)
        self.assertIn("channel=333", log_text)

    def test_interaction_malformed_channel_id_is_denied_without_mirror_lookup_error(self) -> None:
        interaction = FakeInteraction(command_name="ask", channel_id="not-int")

        allowed = _check_interaction_allowed(
            FakeAllowedBot(allowed_user=True, allowed_channel=False),
            interaction,
        )

        log_text = self._log_text()
        self.assertFalse(allowed)
        self.assertIn("slash_ignored command=ask reason=channel_not_allowed", log_text)
        self.assertIn("channel=not-int", log_text)


if __name__ == "__main__":
    _ = unittest.main()
