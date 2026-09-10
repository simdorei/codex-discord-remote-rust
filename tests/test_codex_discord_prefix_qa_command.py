import unittest
from dataclasses import dataclass

import codex_discord_prefix_qa_command as prefix_qa


@dataclass(frozen=True)
class FakeChannel:
    id: int = 222


@dataclass(frozen=True)
class FakeMessage:
    channel: FakeChannel

    @classmethod
    def make(cls, *, channel_id: int = 222) -> "FakeMessage":
        return cls(channel=FakeChannel(channel_id))


class PrefixQaCommandTests(unittest.IsolatedAsyncioTestCase):
    def make_deps(
        self,
        *,
        enabled: bool = True,
        output: str = "qa ok",
        fail: bool = False,
    ) -> tuple[prefix_qa.PrefixQaCommandDeps[object], list[str], list[tuple[object, object]], list[str]]:
        sent: list[str] = []
        calls: list[tuple[object, object]] = []
        logs: list[str] = []

        async def send_chunks(target: object, text: str, *, context: str = "send_chunks") -> int:
            _ = target
            sent.append(f"{context}:{text}")
            return len(text)

        async def run_discord_button_qa(bot: object, message: object) -> str:
            calls.append((bot, message))
            if fail:
                raise RuntimeError("qa failed")
            return output

        deps = prefix_qa.PrefixQaCommandDeps(
            send_chunks=send_chunks,
            qa_commands_enabled=lambda: enabled,
            run_discord_button_qa=run_discord_button_qa,
            log_line=logs.append,
        )
        return deps, sent, calls, logs

    async def test_dispatches_enabled_default_qa_buttons(self) -> None:
        deps, sent, calls, logs = self.make_deps(output="qa output")
        message = FakeMessage.make()
        fake_bot = object()

        handled = await prefix_qa.handle_prefix_qa_command("qa", "", message, fake_bot, deps=deps)

        self.assertTrue(handled)
        self.assertEqual(calls, [(fake_bot, message)])
        self.assertEqual(sent, ["prefix_qa_start:Discord button QA started.", "send_chunks:qa output"])
        self.assertEqual(logs, [])

    async def test_preserves_disabled_usage_alias_failure_and_unhandled(self) -> None:
        message = FakeMessage.make()
        fake_bot = object()

        deps, sent, calls, logs = self.make_deps()
        self.assertFalse(await prefix_qa.handle_prefix_qa_command("where", "", message, fake_bot, deps=deps))
        self.assertEqual(sent, [])
        self.assertEqual(calls, [])

        deps, sent, calls, logs = self.make_deps(enabled=False)
        self.assertTrue(await prefix_qa.handle_prefix_qa_command("qa", "", message, fake_bot, deps=deps))
        self.assertEqual(
            sent,
            ["prefix_qa_disabled:Discord QA commands are disabled. Set DISCORD_ENABLE_QA_COMMANDS=1 to enable them."],
        )
        self.assertEqual(calls, [])
        self.assertEqual(logs, [])

        deps, sent, calls, logs = self.make_deps()
        self.assertTrue(await prefix_qa.handle_prefix_qa_command("qa", "bad", message, fake_bot, deps=deps))
        self.assertEqual(sent, ["prefix_qa_usage:Usage: !qa buttons"])
        self.assertEqual(calls, [])

        deps, sent, calls, logs = self.make_deps(output="qa alias")
        self.assertTrue(await prefix_qa.handle_prefix_qa_command("qa", "button", message, fake_bot, deps=deps))
        self.assertEqual(calls, [(fake_bot, message)])
        self.assertEqual(sent, ["prefix_qa_start:Discord button QA started.", "send_chunks:qa alias"])

        deps, sent, calls, logs = self.make_deps(fail=True)
        self.assertTrue(await prefix_qa.handle_prefix_qa_command("qa", "buttons", message, fake_bot, deps=deps))
        self.assertEqual(calls, [(fake_bot, message)])
        self.assertEqual(sent, ["prefix_qa_start:Discord button QA started.", "send_chunks:Discord button QA failed\n\nERROR: qa failed"])
        self.assertTrue(any(line.startswith("button_qa_failed\n") for line in logs))


if __name__ == "__main__":
    _ = unittest.main()
