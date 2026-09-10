from __future__ import annotations

import unittest
from dataclasses import dataclass
from types import ModuleType

from codex_discord_bot_skill_slash_adapter_runtime import BotSkillSlashAdapterRuntime


@dataclass(frozen=True, slots=True)
class PromptChannel:
    id: int = 111


@dataclass(frozen=True, slots=True)
class MessageableChannel:
    id: int = 222

    async def send(self, content: str) -> None:
        _ = content


@dataclass(frozen=True, slots=True)
class PromptUser:
    id: int = 333
    bot: bool = False


class ResolverModule(ModuleType):
    def __init__(self, resolved_channel: object) -> None:
        super().__init__("skill_slash_adapter_test")
        self.resolved_channel: object = resolved_channel
        self.calls: list[object] = []

    def require_discord_messageable(self, channel: object) -> object:
        self.calls.append(channel)
        return self.resolved_channel


class SkillSlashAdapterRuntimeTests(unittest.TestCase):
    def test_source_message_makers_use_resolved_messageable_channel(self) -> None:
        raw_channel = PromptChannel()
        resolved_channel = MessageableChannel()
        user = PromptUser()
        module = ResolverModule(resolved_channel)
        runtime = BotSkillSlashAdapterRuntime(module)

        skill_source = runtime.make_skill_slash_source_message(raw_channel, user)
        ask_source = runtime.make_slash_ask_source_message(raw_channel, user)

        self.assertEqual(module.calls, [raw_channel, raw_channel])
        self.assertIs(skill_source.channel, resolved_channel)
        self.assertIs(ask_source.channel, resolved_channel)

    def test_source_message_makers_reject_invalid_resolver_result(self) -> None:
        raw_channel = PromptChannel()
        user = PromptUser()

        for invalid_channel in (object(), PromptChannel()):
            with self.subTest(invalid_type=type(invalid_channel).__name__):
                runtime = BotSkillSlashAdapterRuntime(ResolverModule(invalid_channel))

                with self.assertRaises(TypeError):
                    _ = runtime.make_skill_slash_source_message(raw_channel, user)
                with self.assertRaises(TypeError):
                    _ = runtime.make_slash_ask_source_message(raw_channel, user)


if __name__ == "__main__":
    _ = unittest.main()
