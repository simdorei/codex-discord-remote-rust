from __future__ import annotations

import unittest
from dataclasses import dataclass

import codex_discord_busy_interaction_runtime as busy_runtime
import codex_discord_delivery_state as discord_delivery_state


@dataclass(frozen=True, slots=True)
class FakeChannel:
    id: int = 1

    async def send(self, content: str) -> None:
        _ = content


class FakeResponse:
    async def send_message(self, content: str, *, ephemeral: bool = False) -> None:
        _ = (content, ephemeral)


@dataclass(frozen=True, slots=True)
class FakeCommand:
    name: str


@dataclass(frozen=True, slots=True)
class FakeInteraction:
    response: FakeResponse
    command: FakeCommand | None
    channel_id: int


class BusyInteractionRuntimeTests(unittest.IsolatedAsyncioTestCase):
    async def test_busy_wrappers_delegate_to_configured_functions(self) -> None:
        calls: list[str] = []

        async def send_direct_followup(
            interaction: discord_delivery_state.InteractionLike,
            content: str,
            *,
            log_prefix: str,
            context: str,
        ) -> None:
            _ = interaction
            calls.append(f"followup:{content}:{log_prefix}:{context}")

        async def send_stale(
            channel: discord_delivery_state.Messageable,
            prompt: str,
            target_thread_id: str | None,
            *,
            reason: str,
        ) -> bool:
            _ = channel
            calls.append(f"stale:{prompt}:{target_thread_id}:{reason}")
            return True

        async def send_menu(
            channel: discord_delivery_state.Messageable,
            target_thread_id: str | None,
            output: str,
            *,
            reason: str,
        ) -> bool:
            _ = channel
            calls.append(f"menu:{target_thread_id}:{output}:{reason}")
            return True

        async def send_ack(
            channel: discord_delivery_state.Messageable,
            prompt: str,
            target_thread_id: str | None,
        ) -> bool:
            _ = channel
            calls.append(f"ack:{prompt}:{target_thread_id}")
            return True

        runtime = busy_runtime.BusyInteractionRuntime(
            send_direct_followup=send_direct_followup,
            send_stale_busy_steer_block_message=send_stale,
            send_codex_app_menu_if_available=send_menu,
            send_steering_start_ack=send_ack,
        )
        channel = FakeChannel()

        await runtime.send_busy_direct_followup(
            FakeInteraction(FakeResponse(), FakeCommand("cmd"), 123),
            "hello",
            log_prefix="busy",
            context="ctx",
        )
        self.assertTrue(
            await runtime.send_busy_stale_block_message(
                channel,
                "prompt",
                "thread",
                reason="stale",
            )
        )
        self.assertTrue(
            await runtime.send_busy_codex_app_menu_if_available(
                channel,
                "thread",
                "out",
                reason="menu",
            )
        )
        await runtime.send_persistent_busy_steering_start_ack(channel, "prompt", "thread")

        self.assertEqual(
            calls,
            [
                "followup:hello:busy:ctx",
                "stale:prompt:thread:stale",
                "menu:thread:out:menu",
                "ack:prompt:thread",
            ],
        )


if __name__ == "__main__":
    _ = unittest.main()
