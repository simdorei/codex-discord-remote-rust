from __future__ import annotations

import contextlib
import io
import tempfile
import unittest
import urllib.request
from pathlib import Path
from types import TracebackType
from unittest.mock import patch

import setup_discord_bot as setup


class FakeResponse:
    def __init__(self, payload: bytes) -> None:
        self.payload = payload

    def __enter__(self) -> FakeResponse:
        return self

    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        traceback: TracebackType | None,
    ) -> bool | None:
        _ = (exc_type, exc, traceback)
        return None

    def read(self) -> bytes:
        return self.payload


class SetupDiscordBotTests(unittest.TestCase):
    def test_invite_url_uses_expected_permission_bits(self) -> None:
        permissions = setup.get_default_bot_permissions()

        self.assertEqual(permissions, 328565115968)
        self.assertEqual(
            setup.new_discord_bot_invite_url("123", permissions),
            "https://discord.com/oauth2/authorize?client_id=123&scope=bot%20applications.commands&permissions=328565115968",
        )

    def test_env_file_update_preserves_token_equals(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            env_path = Path(temp_dir) / ".env"
            env_path.write_text("OTHER=value\nDISCORD_BOT_TOKEN=old\n", encoding="utf-8")

            setup.set_env_file_value(env_path, "DISCORD_BOT_TOKEN", "abc=def")

            self.assertEqual(env_path.read_text(encoding="utf-8"), "OTHER=value\nDISCORD_BOT_TOKEN=abc=def\n")

    def test_configure_general_channel_id_merges_allowed_and_sets_startup_when_missing(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            env_path = Path(temp_dir) / ".env"
            env_path.write_text("DISCORD_ALLOWED_CHANNEL_IDS=111\nDISCORD_STARTUP_CHANNEL_ID=\n", encoding="utf-8")

            setup.configure_general_channel_id(env_path, "222")
            setup.configure_general_channel_id(env_path, "222")

            self.assertEqual(
                env_path.read_text(encoding="utf-8"),
                "DISCORD_ALLOWED_CHANNEL_IDS=111,222\nDISCORD_STARTUP_CHANNEL_ID=222\n",
            )

    def test_configure_general_channel_id_rejects_non_numeric_channel_id(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            with self.assertRaisesRegex(setup.DiscordChannelIdError, "digits only"):
                setup.configure_general_channel_id(Path(temp_dir) / ".env", "general")

    def test_fetch_discord_application_uses_bot_authorization_header(self) -> None:
        requests: list[urllib.request.Request] = []

        def fake_urlopen(request: urllib.request.Request, *, timeout: float) -> FakeResponse:
            self.assertEqual(timeout, 15.0)
            requests.append(request)
            return FakeResponse(b'{"id":"42","name":"Codex Remote"}')

        application = setup.fetch_discord_application("secret-token", urlopen=fake_urlopen)

        self.assertEqual(application, setup.DiscordApplication(application_id="42", name="Codex Remote"))
        self.assertEqual(requests[0].headers["Authorization"], "Bot secret-token")
        self.assertEqual(
            requests[0].headers["User-agent"],
            "DiscordBot (https://github.com/simdorei/codex-discord-remote, 1.0)",
        )

    def test_dry_run_does_not_write_env_or_print_token(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                exit_code = setup.run(["--repo-root", temp_dir, "--dry-run", "--bot-id", "42"])

            self.assertEqual(exit_code, 0)
            self.assertFalse((Path(temp_dir) / ".env").exists())
            self.assertIn("client_id=42", output.getvalue())
            self.assertNotIn("DISCORD_BOT_TOKEN=", output.getvalue())

    def test_run_saves_general_channel_id_after_token_check(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            with (
                patch.object(setup.getpass, "getpass", return_value="secret-token"),
                patch.object(
                    setup,
                    "fetch_discord_application",
                    return_value=setup.DiscordApplication(application_id="42", name="Codex Remote"),
                ),
                patch("builtins.input", return_value="222"),
                contextlib.redirect_stdout(io.StringIO()),
            ):
                exit_code = setup.run(["--repo-root", temp_dir])

            self.assertEqual(exit_code, 0)
            self.assertEqual(
                (Path(temp_dir) / ".env").read_text(encoding="utf-8"),
                "DISCORD_BOT_TOKEN=secret-token\nDISCORD_ALLOWED_CHANNEL_IDS=222\nDISCORD_STARTUP_CHANNEL_ID=222\n",
            )


if __name__ == "__main__":
    unittest.main()
