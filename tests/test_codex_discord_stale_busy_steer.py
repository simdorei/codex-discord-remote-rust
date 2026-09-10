from __future__ import annotations

from dataclasses import dataclass
from typing import cast
import unittest

import codex_discord_stale_busy_steer as stale_busy_steer


@dataclass(frozen=True, slots=True)
class FakeChannel:
    id: int = 222


class StaleBusySteerTests(unittest.IsolatedAsyncioTestCase):
    async def test_send_stale_busy_steer_block_message_sends_and_logs(self) -> None:
        sent: list[tuple[int, str]] = []
        logs: list[str] = []

        async def send_chunks(channel: stale_busy_steer.StaleBusySteerChannel, content: str) -> int:
            sent.append((cast(FakeChannel, channel).id, content))
            return len(content)

        deps = stale_busy_steer.StaleBusySteerBlockDeps(
            get_block_info=lambda target_thread_id: ("resolved-thread", "project:1", 12.3)
            if target_thread_id == "thread-1"
            else None,
            build_message=lambda prompt, *, target_ref, age_seconds: (
                f"stale message prompt={prompt} target={target_ref} age={age_seconds:.1f}"
            ),
            send_chunks=send_chunks,
            log=logs.append,
            format_log_text_len=lambda prompt: len(prompt),
        )

        handled = await stale_busy_steer.send_stale_busy_steer_block_message(
            FakeChannel(),
            "please steer",
            "thread-1",
            reason="steer_busy_failure",
            deps=deps,
        )

        self.assertTrue(handled)
        self.assertEqual(sent, [(222, "stale message prompt=please steer target=project:1 age=12.3")])
        self.assertEqual(
            logs,
            ["stale_busy_steer_blocked reason=steer_busy_failure target=resolved-thread age_sec=12.3 prompt_len=12"],
        )

    async def test_send_stale_busy_steer_block_message_returns_false_without_info(self) -> None:
        sent: list[str] = []

        async def send_chunks(channel: stale_busy_steer.StaleBusySteerChannel, content: str) -> int:
            _ = channel
            sent.append(content)
            return len(content)

        handled = await stale_busy_steer.send_stale_busy_steer_block_message(
            FakeChannel(),
            "please steer",
            None,
            reason="steer_busy_failure",
            deps=stale_busy_steer.StaleBusySteerBlockDeps(
                get_block_info=lambda target_thread_id: None,
                build_message=lambda prompt, *, target_ref, age_seconds: "unused",
                send_chunks=send_chunks,
                log=lambda line: None,
                format_log_text_len=lambda prompt: len(prompt),
            ),
        )

        self.assertFalse(handled)
        self.assertEqual(sent, [])


if __name__ == "__main__":
    _ = unittest.main()
