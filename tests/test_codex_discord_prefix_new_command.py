import unittest
from dataclasses import dataclass

import codex_discord_prefix_new_command as prefix_new


@dataclass(frozen=True, slots=True)
class FakeChannel:
    id: int = 222


@dataclass(frozen=True, slots=True)
class FakeMessage:
    channel: FakeChannel

    @classmethod
    def make(cls, *, channel_id: int = 222) -> "FakeMessage":
        return cls(channel=FakeChannel(channel_id))


class PrefixNewCommandTests(unittest.IsolatedAsyncioTestCase):
    def make_deps(
        self,
        *,
        output: str = "new ok",
    ) -> tuple[prefix_new.PrefixNewCommandDeps, list[str], list[tuple[prefix_new.BotLike, int | None, str]]]:
        sent: list[str] = []
        calls: list[tuple[prefix_new.BotLike, int | None, str]] = []

        async def send_chunks(target: prefix_new.ChannelLike, text: str, *, context: str = "send_chunks") -> int:
            _ = target
            sent.append(f"{context}:{text}")
            return len(text)

        async def run_discord_new_thread(
            bot: prefix_new.BotLike,
            channel_id: int | None,
            prompt: str,
        ) -> tuple[int, str]:
            calls.append((bot, channel_id, prompt))
            return 0, output

        deps = prefix_new.PrefixNewCommandDeps(
            send_chunks=send_chunks,
            run_discord_new_thread=run_discord_new_thread,
        )
        return deps, sent, calls

    async def test_dispatches_new_prompt(self) -> None:
        deps, sent, calls = self.make_deps(output="new thread")
        message = FakeMessage.make(channel_id=222)
        fake_bot = object()

        handled = await prefix_new.handle_prefix_new_command("new", "start here", message, fake_bot, deps=deps)

        self.assertTrue(handled)
        self.assertEqual(calls, [(fake_bot, 222, "start here")])
        self.assertEqual(sent, ["send_chunks:new thread"])

    async def test_preserves_usage_and_unhandled_fallthrough(self) -> None:
        deps, sent, calls = self.make_deps()
        message = FakeMessage.make()
        fake_bot = object()

        self.assertFalse(await prefix_new.handle_prefix_new_command("qa", "buttons", message, fake_bot, deps=deps))
        self.assertEqual(sent, [])
        self.assertEqual(calls, [])

        self.assertTrue(await prefix_new.handle_prefix_new_command("new", "", message, fake_bot, deps=deps))
        self.assertEqual(sent, ["prefix_new_usage:Usage: !new <prompt>"])
        self.assertEqual(calls, [])


if __name__ == "__main__":
    _ = unittest.main()
