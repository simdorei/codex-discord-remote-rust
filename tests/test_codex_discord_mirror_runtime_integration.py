from __future__ import annotations

# pyright: reportAssignmentType=false, reportAttributeAccessIssue=false, reportUnknownMemberType=false, reportUnknownVariableType=false
from dataclasses import dataclass
from typing import Never, override
import unittest

import discord

import codex_discord_bot as bot
import codex_desktop_bridge as bridge
from codex_thread_models import ThreadInfo


@dataclass(frozen=True, slots=True)
class MissingGuild:
    id: int = 0


@dataclass(frozen=True, slots=True)
class MissingGuildBot:
    guild_id: int | None = None
    guilds: tuple[discord.Guild, ...] = ()

    def get_guild(self, guild_id: int) -> None:
        _ = guild_id
        return None

    def get_channel(self, channel_id: int) -> None:
        _ = channel_id
        return None

    async def fetch_channel(self, channel_id: int) -> Never:
        _ = channel_id
        raise AssertionError("unexpected bot channel fetch")


@dataclass(frozen=True, slots=True)
class MirrorCategory:
    id: int


@dataclass(frozen=True, slots=True)
class WrongPreferredChannel:
    id: int


class ChannelFetchError(RuntimeError):
    @override
    def __str__(self) -> str:
        return "channel boom"


class PreferredChannelBug(TypeError):
    @override
    def __str__(self) -> str:
        return "preferred channel bug"


class DiscordMirrorRuntimeIntegrationTests(unittest.IsolatedAsyncioTestCase):
    async def test_mirror_single_codex_thread_rejects_wrong_preferred_project_channel_type(self) -> None:
        original_get_mirror_guild = bot.get_mirror_guild
        original_get_category = bot.get_or_create_mirror_category
        original_choose_thread = bridge.choose_thread

        class FakeGuild:
            def get_channel(self, channel_id: int) -> None:
                _ = channel_id
                return None

            async def fetch_channel(self, channel_id: int) -> WrongPreferredChannel:
                return WrongPreferredChannel(channel_id)

        async def fake_get_mirror_guild(fake_bot: object) -> FakeGuild:
            _ = fake_bot
            return FakeGuild()

        async def fake_get_category(guild: FakeGuild) -> MirrorCategory:
            _ = guild
            return MirrorCategory(999)

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

        try:
            bot.get_mirror_guild = fake_get_mirror_guild
            bot.get_or_create_mirror_category = fake_get_category
            bridge.choose_thread = fake_choose_thread

            with self.assertRaisesRegex(
                bot.PreferredMirrorProjectChannelTypeError,
                r"Preferred mirror project channel 777 is WrongPreferredChannel, not TextChannel.",
            ):
                _ = await bot.mirror_single_codex_thread(
                    MissingGuildBot(),
                    "thread-new",
                    preferred_project_channel_id=777,
                )
        finally:
            bot.get_mirror_guild = original_get_mirror_guild
            bot.get_or_create_mirror_category = original_get_category
            bridge.choose_thread = original_choose_thread

    async def test_mirror_single_codex_thread_type_error_is_not_preferred_channel_unavailable(self) -> None:
        original_get_mirror_guild = bot.get_mirror_guild
        original_get_category = bot.get_or_create_mirror_category
        original_choose_thread = bridge.choose_thread

        class FakeGuild:
            def get_channel(self, channel_id: int) -> None:
                _ = channel_id
                return None

            async def fetch_channel(self, channel_id: int) -> MissingGuild:
                _ = channel_id
                raise PreferredChannelBug()

        async def fake_get_mirror_guild(fake_bot: object) -> FakeGuild:
            _ = fake_bot
            return FakeGuild()

        async def fake_get_category(guild: FakeGuild) -> MirrorCategory:
            _ = guild
            return MirrorCategory(999)

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

        try:
            bot.get_mirror_guild = fake_get_mirror_guild
            bot.get_or_create_mirror_category = fake_get_category
            bridge.choose_thread = fake_choose_thread

            with self.assertRaisesRegex(TypeError, "preferred channel bug"):
                _ = await bot.mirror_single_codex_thread(
                    MissingGuildBot(),
                    "thread-new",
                    preferred_project_channel_id=777,
                )
        finally:
            bot.get_mirror_guild = original_get_mirror_guild
            bot.get_or_create_mirror_category = original_get_category
            bridge.choose_thread = original_choose_thread

    async def test_mirror_single_codex_thread_surfaces_missing_preferred_project_channel(self) -> None:
        original_get_mirror_guild = bot.get_mirror_guild
        original_get_category = bot.get_or_create_mirror_category
        original_choose_thread = bridge.choose_thread

        class FakeGuild:
            def get_channel(self, channel_id: int) -> None:
                _ = channel_id
                return None

            async def fetch_channel(self, channel_id: int) -> MissingGuild:
                _ = channel_id
                raise ChannelFetchError()

        async def fake_get_mirror_guild(fake_bot: object) -> FakeGuild:
            _ = fake_bot
            return FakeGuild()

        async def fake_get_category(guild: FakeGuild) -> MirrorCategory:
            _ = guild
            return MirrorCategory(999)

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

        try:
            bot.get_mirror_guild = fake_get_mirror_guild
            bot.get_or_create_mirror_category = fake_get_category
            bridge.choose_thread = fake_choose_thread

            with self.assertRaisesRegex(
                bot.PreferredMirrorProjectChannelUnavailableError,
                r"Preferred mirror project channel 777 .*ChannelFetchError: channel boom",
            ):
                _ = await bot.mirror_single_codex_thread(
                    MissingGuildBot(),
                    "thread-new",
                    preferred_project_channel_id=777,
                )
        finally:
            bot.get_mirror_guild = original_get_mirror_guild
            bot.get_or_create_mirror_category = original_get_category
            bridge.choose_thread = original_choose_thread

    async def test_get_mirror_guild_raises_typed_error_when_unavailable(self) -> None:
        with self.assertRaisesRegex(
            bot.MirrorGuildUnavailableError,
            "Discord guild is not available yet.",
        ):
            _ = await bot.get_mirror_guild(MissingGuildBot())


if __name__ == "__main__":
    _ = unittest.main()
