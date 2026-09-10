from __future__ import annotations

from types import ModuleType
import unittest
from pathlib import Path

from codex_discord_bot_module_context import (
    RuntimeBindingCollisionError,
    RuntimeStateBindingError,
    install_runtime_bindings,
    update_runtime_state,
)


ROOT = Path(__file__).resolve().parents[1]


class BotModuleContextTests(unittest.TestCase):
    def test_binding_owner_can_refresh_its_own_values(self) -> None:
        module = ModuleType("test_runtime_module")

        install_runtime_bindings(module, owner="owner-a", values={"handler": "first"})
        install_runtime_bindings(module, owner="owner-a", values={"handler": "second"})

        self.assertEqual(module.handler, "second")

    def test_different_owner_cannot_silently_replace_a_binding(self) -> None:
        module = ModuleType("test_runtime_module")
        install_runtime_bindings(module, owner="owner-a", values={"handler": "first"})

        with self.assertRaises(RuntimeBindingCollisionError):
            install_runtime_bindings(module, owner="owner-b", values={"handler": "second"})

        self.assertEqual(module.handler, "first")

    def test_runtime_state_updates_only_predeclared_values(self) -> None:
        module = ModuleType("test_runtime_module")
        module.stopping = False

        update_runtime_state(module, {"stopping": True})

        self.assertTrue(module.stopping)
        with self.assertRaises(RuntimeStateBindingError):
            update_runtime_state(module, {"missing": True})

    def test_wiring_modules_do_not_mutate_module_attributes_directly(self) -> None:
        offenders = []
        for path in ROOT.glob("codex_discord_bot_*runtime.py"):
            if "setattr(self.module" in path.read_text(encoding="utf-8"):
                offenders.append(path.name)

        self.assertEqual(offenders, [])


if __name__ == "__main__":
    _ = unittest.main()
