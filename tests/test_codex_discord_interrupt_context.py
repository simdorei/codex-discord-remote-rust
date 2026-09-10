from __future__ import annotations

import asyncio
import unittest

import codex_discord_interrupt_context as interrupt_context


class DiscordInterruptContextTests(unittest.IsolatedAsyncioTestCase):
    async def test_remote_stop_scope_propagates_to_worker_and_resets(self) -> None:
        self.assertFalse(interrupt_context.is_discord_remote_stop())

        with interrupt_context.discord_remote_stop_scope():
            in_worker = await asyncio.to_thread(interrupt_context.is_discord_remote_stop)
            self.assertTrue(in_worker)

        self.assertFalse(interrupt_context.is_discord_remote_stop())


if __name__ == "__main__":
    _ = unittest.main()
