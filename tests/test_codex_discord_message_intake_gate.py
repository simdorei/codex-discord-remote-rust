from __future__ import annotations

from dataclasses import dataclass
import unittest

import codex_discord_message_intake_gate as intake_gate


@dataclass(frozen=True, slots=True)
class FakeAuthor:
    id: int


@dataclass(frozen=True, slots=True)
class FakeCategory:
    name: str


@dataclass(frozen=True, slots=True)
class FakeChannel:
    id: int = 123
    parent_id: int | None = None
    category: FakeCategory | None = None
    parent: FakeParent | None = None


@dataclass(frozen=True, slots=True)
class FakeParent:
    category: FakeCategory | None = None


@dataclass(frozen=True, slots=True)
class FakeMessage:
    channel: FakeChannel
    author: FakeAuthor


class GateFixture:
    def __init__(
        self,
        *,
        allowed_channel: bool = True,
        allowed_user: bool = True,
        stopping: bool = False,
        bot_bridge_mention: bool = False,
    ) -> None:
        self.logs: list[str] = []
        self.notices: list[int] = []
        self.mention_calls: int = 0
        self.allowed_channel = allowed_channel
        self.allowed_user = allowed_user
        self.stopping = stopping
        self.bot_bridge_mention = bot_bridge_mention

    def deps(self) -> intake_gate.MessageIntakeGateDeps[FakeChannel, FakeMessage]:
        return intake_gate.MessageIntakeGateDeps(
            is_allowed_message_channel=lambda channel: self.allowed_channel,
            is_bot_authored_bridge_mention=self.is_bot_authored_bridge_mention,
            is_allowed_user=lambda _user_id: self.allowed_user,
            is_stopping=lambda: self.stopping,
            send_restarting_notice=self.send_restarting_notice,
            log=self.logs.append,
        )

    def is_bot_authored_bridge_mention(self, message: FakeMessage) -> bool:
        self.mention_calls += 1
        return self.bot_bridge_mention

    async def send_restarting_notice(self, target: FakeChannel) -> None:
        self.notices.append(target.id)


class MessageIntakeGateTests(unittest.IsolatedAsyncioTestCase):
    async def test_channel_denial_handles_without_bridge_mention_check(self) -> None:
        fixture = GateFixture(allowed_channel=False)
        message = FakeMessage(
            channel=FakeChannel(id=555, parent_id=777, category=FakeCategory("ops")),
            author=FakeAuthor(42),
        )

        result = await intake_gate.gate_discord_message(message, message_channel=message.channel, deps=fixture.deps())

        self.assertTrue(result.handled)
        self.assertFalse(result.bot_bridge_mention)
        self.assertEqual(fixture.mention_calls, 0)
        self.assertIn("ignored_message reason=channel_not_allowed chat=555", fixture.logs[0])
        self.assertIn("category=ops", fixture.logs[0])

    async def test_stopping_sends_restart_notice_after_bridge_mention_check(self) -> None:
        fixture = GateFixture(stopping=True, bot_bridge_mention=True)
        message = FakeMessage(channel=FakeChannel(id=555), author=FakeAuthor(42))

        result = await intake_gate.gate_discord_message(message, message_channel=message.channel, deps=fixture.deps())

        self.assertTrue(result.handled)
        self.assertTrue(result.bot_bridge_mention)
        self.assertEqual(fixture.notices, [555])
        self.assertEqual(fixture.mention_calls, 1)
        self.assertIn("message_rejected reason=bot_stopping chat=555 user=42", fixture.logs)


if __name__ == "__main__":
    unittest.main()
