import unittest
from dataclasses import dataclass

import codex_discord_prefix_status_commands as prefix_status

PrefixStatusCallValue = int | str | tuple[int | None, bool, int] | tuple[int | None, int] | None


@dataclass(frozen=True, slots=True)
class FakeAuthor:
    id: int = 333


@dataclass(frozen=True, slots=True)
class FakeChannel:
    id: int = 222
    name: str = "ops"


@dataclass(frozen=True, slots=True)
class FakeNamelessChannel:
    id: int = 222


@dataclass(frozen=True, slots=True)
class FakeGuild:
    id: int = 111


@dataclass(frozen=True, slots=True)
class FakeMessage:
    channel: FakeChannel
    author: FakeAuthor
    guild: FakeGuild | None

    @classmethod
    def make(
        cls,
        *,
        channel_id: int = 222,
        author_id: int = 333,
        guild_id: int | None = 111,
        channel_name: str = "ops",
    ) -> "FakeMessage":
        guild = None if guild_id is None else FakeGuild(guild_id)
        return cls(
            channel=FakeChannel(channel_id, channel_name),
            author=FakeAuthor(author_id),
            guild=guild,
        )


@dataclass(frozen=True, slots=True)
class FakeNamelessMessage:
    channel: FakeNamelessChannel
    author: FakeAuthor
    guild: None


class PrefixStatusCommandTests(unittest.IsolatedAsyncioTestCase):
    def make_deps(
        self,
    ) -> tuple[prefix_status.PrefixStatusCommandDeps, list[str], list[tuple[str, PrefixStatusCallValue]]]:
        sent: list[str] = []
        calls: list[tuple[str, PrefixStatusCallValue]] = []

        async def send_chunks(target: prefix_status.ChannelLike, text: str, *, context: str = "send_chunks") -> int:
            _ = target
            sent.append(f"{context}:{text}")
            return len(text)

        def build_where(channel_id: int | None) -> str:
            calls.append(("where", channel_id))
            return f"where:{channel_id}"

        def build_context(
            channel_id: int | None = None,
            *,
            all_threads: bool = False,
            limit: int = 10,
        ) -> str:
            calls.append(("context", (channel_id, all_threads, limit)))
            return f"context:{channel_id}:{all_threads}:{limit}"

        def build_context_refresh(channel_id: int | None = None, *, limit: int = 10) -> str:
            calls.append(("context_refresh", (channel_id, limit)))
            return f"context_refresh:{channel_id}:{limit}"

        def clamp_context_refresh_limit(value: str) -> int:
            calls.append(("clamp", value))
            if value == "bad":
                return 12
            return int(value or "10")

        def build_weekly_usage_message(days: int = 7) -> str:
            calls.append(("usage", days))
            return f"usage:{days}"

        async def build_runners_message() -> str:
            calls.append(("runners", None))
            return "runners"

        async def build_system_resources_message() -> str:
            calls.append(("resources", None))
            return "resources"

        deps = prefix_status.PrefixStatusCommandDeps(
            send_chunks=send_chunks,
            build_where_message=build_where,
            build_context_message=build_context,
            build_context_refresh_message=build_context_refresh,
            clamp_context_refresh_limit=clamp_context_refresh_limit,
            build_weekly_usage_message=build_weekly_usage_message,
            build_runners_message=build_runners_message,
            build_system_resources_message=build_system_resources_message,
        )
        return deps, sent, calls

    async def test_dispatches_identity_where_context_usage_runners_and_resources(self) -> None:
        deps, sent, calls = self.make_deps()
        message = FakeMessage.make(channel_id=222, author_id=333, guild_id=111, channel_name="ops")

        self.assertTrue(await prefix_status.handle_prefix_status_command("chatid", "", message, deps=deps))
        self.assertTrue(await prefix_status.handle_prefix_status_command("where", "", message, deps=deps))
        self.assertTrue(await prefix_status.handle_prefix_status_command("ctx", "refresh 5", message, deps=deps))
        self.assertTrue(await prefix_status.handle_prefix_status_command("context", "all", message, deps=deps))
        self.assertTrue(await prefix_status.handle_prefix_status_command("usage", "9", message, deps=deps))
        self.assertTrue(await prefix_status.handle_prefix_status_command("runners", "", message, deps=deps))
        self.assertTrue(await prefix_status.handle_prefix_status_command("resources", "", message, deps=deps))

        self.assertEqual(
            sent,
            [
                (
                    "send_chunks:Discord identity\n"
                    + "guild_id: 111\n"
                    + "channel_id: 222\n"
                    + "user_id: 333\n"
                    + "channel_name: ops\n"
                    + "\n"
                    + "Copy into .env if needed:\n"
                    + "DISCORD_ALLOWED_CHANNEL_IDS=222\n"
                    + "DISCORD_ALLOWED_USER_IDS=333"
                ),
                "send_chunks:where:222",
                "send_chunks:context_refresh:222:5",
                "send_chunks:context:222:True:20",
                "send_chunks:usage:9",
                "send_chunks:runners",
                "send_chunks:resources",
            ],
        )
        self.assertEqual(
            calls,
            [
                ("where", 222),
                ("clamp", "5"),
                ("context_refresh", (222, 5)),
                ("context", (222, True, 20)),
                ("usage", 9),
                ("runners", None),
                ("resources", None),
            ],
        )

    async def test_preserves_edges_aliases_and_unhandled_fallthrough(self) -> None:
        deps, sent, calls = self.make_deps()
        message = FakeMessage.make(guild_id=None)
        message_without_name = FakeNamelessMessage(
            channel=FakeNamelessChannel(222),
            author=FakeAuthor(333),
            guild=None,
        )

        self.assertFalse(await prefix_status.handle_prefix_status_command("new", "prompt", message, deps=deps))

        self.assertTrue(await prefix_status.handle_prefix_status_command("whoami", "", message_without_name, deps=deps))
        self.assertIn("guild_id: -", sent[-1])
        self.assertIn("channel_name: -", sent[-1])

        self.assertTrue(await prefix_status.handle_prefix_status_command("quota", "bad", message, deps=deps))
        self.assertEqual(sent[-1], "prefix_usage_help:Usage: !usage [days]")

        self.assertTrue(await prefix_status.handle_prefix_status_command("limit", "", message, deps=deps))
        self.assertEqual(sent[-1], "send_chunks:usage:7")

        self.assertTrue(await prefix_status.handle_prefix_status_command("context", "recent bad", message, deps=deps))
        self.assertEqual(sent[-1], "send_chunks:context_refresh:222:12")

        self.assertTrue(await prefix_status.handle_prefix_status_command("ctx", "*", message, deps=deps))
        self.assertEqual(sent[-1], "send_chunks:context:222:True:20")

        self.assertTrue(await prefix_status.handle_prefix_status_command("queues", "", message, deps=deps))
        self.assertEqual(sent[-1], "send_chunks:runners")
        self.assertTrue(await prefix_status.handle_prefix_status_command("system", "", message, deps=deps))
        self.assertEqual(sent[-1], "send_chunks:resources")
        self.assertIn(("clamp", "bad"), calls)


if __name__ == "__main__":
    _ = unittest.main()
