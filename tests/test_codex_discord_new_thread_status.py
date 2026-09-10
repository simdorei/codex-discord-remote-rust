import unittest
from unittest import mock

import codex_discord_bot as bot


class DiscordNewThreadStatusTests(unittest.IsolatedAsyncioTestCase):
    def test_format_new_thread_prefix_reports_partial_creation(self) -> None:
        self.assertEqual(
            bot.format_discord_new_thread_prefix(1, "thread-1"),
            "New created but verification failed (exit 1)",
        )
        self.assertEqual(bot.format_discord_new_thread_prefix(1, None), "New failed (exit 1)")

    async def test_run_discord_new_thread_prepares_mirrored_thread_output(self) -> None:
        class FakeDiscordThread:
            id = 123

        class FakeCodexThread:
            id = "thread-1"

        prepared: list[tuple[FakeDiscordThread, str | None]] = []

        async def fake_mirror_single_codex_thread(
            fake_bot: bot.CodexDiscordBot,
            thread_id: str,
            *,
            preferred_project_channel_id: int | None,
        ) -> FakeDiscordThread:
            _ = fake_bot
            self.assertEqual(thread_id, "thread-1")
            self.assertEqual(preferred_project_channel_id, 456)
            return FakeDiscordThread()

        async def fake_prepare(channel: FakeDiscordThread, target_thread_id: str | None) -> bool:
            prepared.append((channel, target_thread_id))
            return True

        output = "selected_thread: thread-1\ntarget_thread: thread-1\n"
        fake_bot: bot.CodexDiscordBot = mock.create_autospec(bot.CodexDiscordBot, instance=True)
        with (
            mock.patch.object(bot, "resolve_discord_new_thread_cwd", return_value="C:/repo"),
            mock.patch.object(bot, "run_bridge_command", return_value=(1, output)),
            mock.patch.object(bot.BRIDGE_MIRROR_STATUS, "choose_thread", return_value=FakeCodexThread()),
            mock.patch.object(bot, "get_project_key", return_value="project"),
            mock.patch.object(bot, "resolve_discord_new_thread_project_channel_id", return_value=456),
            mock.patch.object(bot, "mirror_single_codex_thread", side_effect=fake_mirror_single_codex_thread),
            mock.patch.object(bot, "prepare_mapped_session_mirror_output", side_effect=fake_prepare),
        ):
            exit_code, message = await bot.run_discord_new_thread(fake_bot, 789, "prompt")

        self.assertEqual(exit_code, 1)
        self.assertIn("New created but verification failed", message)
        self.assertEqual(len(prepared), 1)
        self.assertIsInstance(prepared[0][0], FakeDiscordThread)
        self.assertEqual(prepared[0][1], "thread-1")


if __name__ == "__main__":
    _ = unittest.main()
