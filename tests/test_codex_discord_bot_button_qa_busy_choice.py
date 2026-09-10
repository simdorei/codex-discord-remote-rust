from __future__ import annotations

from dataclasses import dataclass, field
from importlib.machinery import ModuleSpec, SourceFileLoader
from types import ModuleType
from typing import cast
import unittest

from codex_discord_bot_button_qa_adapter_runtime import BotButtonQaAdapterRuntime
from codex_discord_bot_button_qa_busy_choice import QaNoStaleBlockModule
import codex_discord_button_qa_lifecycle_cases as lifecycle_cases


@dataclass
class FakeLifecycleResponse:
    messages: list[str] = field(default_factory=list)


@dataclass
class FakeLifecycleInteraction:
    response: FakeLifecycleResponse = field(default_factory=FakeLifecycleResponse)


class QaNoStaleBlockModuleTests(unittest.TestCase):
    def test_preserves_module_identity_metadata_and_delegates_live_attributes(self) -> None:
        source = ModuleType("qa_source", "qa source docs")
        loader = SourceFileLoader("qa_source", "qa_source.py")
        spec = ModuleSpec("qa_source", loader)
        source.__package__ = "qa_package"
        source.__loader__ = loader
        source.__spec__ = spec
        setattr(source, "dynamic_value", "before")

        async def original_sender(*_args: object, **_kwargs: object) -> bool:
            return True

        async def replacement_sender(*_args: object, **_kwargs: object) -> bool:
            return False

        setattr(source, "send_busy_stale_block_message", original_sender)

        proxy = QaNoStaleBlockModule(source, replacement_sender)

        self.assertIsInstance(proxy, ModuleType)
        self.assertEqual(proxy.__name__, source.__name__)
        self.assertEqual(proxy.__doc__, source.__doc__)
        self.assertEqual(proxy.__package__, source.__package__)
        self.assertIs(proxy.__loader__, source.__loader__)
        self.assertIs(proxy.__spec__, source.__spec__)
        self.assertIs(proxy.send_busy_stale_block_message, replacement_sender)
        self.assertIs(cast(object, getattr(source, "send_busy_stale_block_message")), original_sender)
        self.assertEqual(getattr(proxy, "dynamic_value"), "before")

        setattr(source, "dynamic_value", "after")
        self.assertEqual(getattr(proxy, "dynamic_value"), "after")


class BotButtonQaBusyChoiceMixinTests(unittest.IsolatedAsyncioTestCase):
    async def test_concrete_runtime_uses_module_supplied_to_dataclass(self) -> None:
        module = ModuleType("qa_runtime")
        calls: list[tuple[object, str]] = []

        def require_discord_interaction(interaction: object) -> object:
            return interaction

        async def handle_persistent_busy_choice_interaction(interaction: object, custom_id: str) -> bool:
            calls.append((interaction, custom_id))
            return True

        setattr(module, "require_discord_interaction", require_discord_interaction)
        setattr(module, "handle_persistent_busy_choice_interaction", handle_persistent_busy_choice_interaction)
        runtime = BotButtonQaAdapterRuntime(module)
        interaction: lifecycle_cases.BusyChoiceQaInteraction = FakeLifecycleInteraction()

        handled = await runtime.handle_lifecycle_qa_busy_choice_interaction(interaction, "ignore:qa")

        self.assertIs(runtime.module, module)
        self.assertTrue(handled)
        self.assertEqual(calls, [(interaction, "ignore:qa")])


if __name__ == "__main__":
    _ = unittest.main()
