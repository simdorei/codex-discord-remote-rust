from __future__ import annotations

from types import ModuleType
import unittest

from codex_discord_bot_plain_ask_adapter_runtime import BotPlainAskAdapterRuntime


class BotPlainAskAdapterStateMixinTests(unittest.TestCase):
    def test_concrete_runtime_uses_module_from_dataclass_field(self) -> None:
        module = ModuleType("plain_ask_runtime")
        calls: list[tuple[str | None, str]] = []

        def has_recent_codex_app_user_prompt(target_thread_id: str | None, prompt: str) -> bool:
            calls.append((target_thread_id, prompt))
            return True

        setattr(module, "has_recent_codex_app_user_prompt", has_recent_codex_app_user_prompt)
        runtime = BotPlainAskAdapterRuntime(module)

        result = runtime.has_recent_codex_app_user_prompt("thread-1", "hello")

        self.assertIs(runtime.module, module)
        self.assertTrue(result)
        self.assertEqual(calls, [("thread-1", "hello")])


if __name__ == "__main__":
    _ = unittest.main()
