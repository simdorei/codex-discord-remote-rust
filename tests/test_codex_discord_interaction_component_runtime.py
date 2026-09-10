from __future__ import annotations

import unittest
from dataclasses import dataclass

import codex_discord_interaction_component_runtime as component_runtime


@dataclass(frozen=True, slots=True)
class FakeUser:
    id: int


class EditableMessage:
    def __init__(self) -> None:
        self.cleared: bool = False

    async def edit(self, *, view: None = None) -> None:
        _ = view
        self.cleared = True


@dataclass(frozen=True, slots=True)
class FakeInteraction:
    message: EditableMessage | None
    channel_id: int
    user: FakeUser


class FakeChannel:
    id: int = 30

    async def send(self, content: str) -> None:
        _ = content


@dataclass(frozen=True, slots=True)
class FakeChannelInteraction:
    channel: FakeChannel | None
    client: "FakeClient | None"


class FakeClient:
    def __init__(self, channel: FakeChannel | None = None) -> None:
        self.channel: FakeChannel | None = channel

    async def fetch_channel(self, channel_id: int) -> FakeChannel | None:
        _ = channel_id
        return self.channel


class InteractionComponentRuntimeTests(unittest.IsolatedAsyncioTestCase):
    async def test_clear_interaction_message_components_clears_view(self) -> None:
        logs: list[str] = []
        message = EditableMessage()
        runtime = component_runtime.InteractionComponentRuntime(
            delivery_exceptions=(RuntimeError,),
            format_exception=lambda: "trace",
            log=logs.append,
        )

        await runtime.clear_interaction_message_components(
            FakeInteraction(message=message, channel_id=10, user=FakeUser(20)),
            context="unit",
        )

        self.assertTrue(message.cleared)
        self.assertTrue(
            any(
                "component_message_components_cleared context=unit channel=10 user=20"
                in line
                for line in logs
            )
        )

    async def test_clear_interaction_message_components_ignores_missing_message(self) -> None:
        logs: list[str] = []
        runtime = component_runtime.InteractionComponentRuntime(
            delivery_exceptions=(RuntimeError,),
            format_exception=lambda: "trace",
            log=logs.append,
        )

        await runtime.clear_interaction_message_components(
            FakeInteraction(message=None, channel_id=10, user=FakeUser(20)),
            context="unit",
        )

        self.assertEqual(logs, [])

    async def test_resolve_interaction_channel_uses_existing_channel(self) -> None:
        runtime = component_runtime.InteractionComponentRuntime(
            delivery_exceptions=(RuntimeError,),
            format_exception=lambda: "trace",
            log=lambda message: None,
        )
        channel = FakeChannel()

        resolved = await runtime.resolve_interaction_channel(
            FakeChannelInteraction(channel=channel, client=None),
            30,
        )

        self.assertIs(resolved, channel)

    async def test_resolve_interaction_channel_fetches_missing_channel(self) -> None:
        runtime = component_runtime.InteractionComponentRuntime(
            delivery_exceptions=(RuntimeError,),
            format_exception=lambda: "trace",
            log=lambda message: None,
        )
        channel = FakeChannel()

        resolved = await runtime.resolve_interaction_channel(
            FakeChannelInteraction(channel=None, client=FakeClient(channel)),
            30,
        )

        self.assertIs(resolved, channel)


if __name__ == "__main__":
    _ = unittest.main()
