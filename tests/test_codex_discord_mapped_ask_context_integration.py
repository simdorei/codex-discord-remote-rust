from __future__ import annotations

from pathlib import Path
from typing import Protocol, cast
import tempfile
import unittest

import codex_desktop_bridge as bridge
import codex_discord_bot as bot

from tests.test_codex_discord_ask_busy_failure_integration import run_prompt_and_send
from tests.test_codex_discord_bot import EnvPatch, FakeTarget


class ThreadIdOnly:
    def __init__(self, thread_id: str | None) -> None:
        self.id: str | None = thread_id


class ContextUsage:
    model_context_window: int = 258400
    peak_input_tokens: int = 236419
    last_total_tokens: int = 0


class ChooseThreadForContext(Protocol):
    def __call__(self, thread_id: str | None = None, cwd: str | None = None) -> ThreadIdOnly:
        ...


class GetThreadContextUsage(Protocol):
    def __call__(self, thread: ThreadIdOnly) -> ContextUsage:
        ...


class DiscordMappedAskContextIntegrationTests(unittest.IsolatedAsyncioTestCase):
    async def test_context_exhausted_mapped_ask_blocks_before_transport(self) -> None:
        original_resolve_target_ref = bot.resolve_target_ref
        original_run_transport = bot.run_transport_prompt_no_wait
        original_run_ask_stream = bot.run_ask_stream
        original_choose_thread = cast(ChooseThreadForContext, getattr(bridge, "choose_thread"))
        original_get_context_usage = cast(GetThreadContextUsage, getattr(bridge, "get_thread_context_usage"))
        calls: list[str] = []
        try:
            bot.resolve_target_ref = lambda target_thread_id: (target_thread_id, "taxlab:2")

            def fake_choose_thread(
                thread_id: str | None = None,
                cwd: str | None = None,
            ) -> ThreadIdOnly:
                _ = cwd
                return ThreadIdOnly(thread_id)

            def fake_get_thread_context_usage(thread: ThreadIdOnly) -> ContextUsage:
                _ = thread
                return ContextUsage()

            def fail_transport(prompt: str, target_thread_id: str | None) -> tuple[int, str]:
                _ = prompt, target_thread_id
                calls.append("transport")
                raise AssertionError("context-exhausted ask must not start no-wait transport")

            def fail_stream(
                prompt: str,
                relay: bot.DiscordAskRelay,
                *,
                force_while_busy: bool = False,
                wait: bool = True,
                target_thread_id: str | None = None,
            ) -> tuple[int, str]:
                _ = prompt, relay, force_while_busy, wait, target_thread_id
                calls.append("stream")
                raise AssertionError("context-exhausted ask must not start stream transport")

            setattr(bridge, "choose_thread", fake_choose_thread)
            setattr(bridge, "get_thread_context_usage", fake_get_thread_context_usage)
            bot.run_transport_prompt_no_wait = fail_transport
            bot.run_ask_stream = fail_stream

            with tempfile.TemporaryDirectory() as temp_dir:
                log_path = Path(temp_dir) / "discord-smoke.log"
                channel = FakeTarget(channel_id=222)
                with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                    await run_prompt_and_send()(
                        channel,
                        "please run",
                        ack_sent=True,
                        target_thread_id="thread-1",
                    )
                log_text = log_path.read_text(encoding="utf-8")

            self.assertEqual(calls, [])
            self.assertEqual(len(channel.messages), 1)
            self.assertIn("no-visible-reply state", channel.messages[0][0])
            self.assertIn("236.4k/258.4k", channel.messages[0][0])
            self.assertIn("!archive taxlab:2", channel.messages[0][0])
            self.assertIn("!mirror sync", channel.messages[0][0])
            self.assertIn("ask_blocked_context_exhausted target=thread-1", log_text)
        finally:
            bot.resolve_target_ref = original_resolve_target_ref
            bot.run_transport_prompt_no_wait = original_run_transport
            bot.run_ask_stream = original_run_ask_stream
            setattr(bridge, "choose_thread", original_choose_thread)
            setattr(bridge, "get_thread_context_usage", original_get_context_usage)


if __name__ == "__main__":
    _ = unittest.main()
