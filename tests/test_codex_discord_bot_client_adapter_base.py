from __future__ import annotations

from types import ModuleType
import unittest

from codex_discord_bot_client_adapter_runtime import BotClientAdapterRuntime


class BotClientAdapterBaseTests(unittest.TestCase):
    def test_concrete_runtime_uses_module_from_dataclass_field(self) -> None:
        module = ModuleType("client_adapter_runtime")

        runtime = BotClientAdapterRuntime(module)

        self.assertIs(runtime.module, module)


if __name__ == "__main__":
    _ = unittest.main()
