from __future__ import annotations

import ast
import unittest
from pathlib import Path

import codex_discord_bot as bot
import codex_discord_bot_history_runtime as history_runtime
import codex_discord_bot_message_runtime as message_runtime
import codex_discord_bot_socket_runtime as socket_runtime
import codex_discord_processed_message_runtime as processed_message_runtime
import codex_discord_text as discord_text


def _public_stub_names() -> set[str]:
    stub_path = Path(__file__).resolve().parents[1] / "codex_discord_bot_type_exports.pyi"
    tree = ast.parse(stub_path.read_text(encoding="utf-8"))
    names: set[str] = set()
    for node in tree.body:
        if isinstance(node, (ast.AsyncFunctionDef, ast.ClassDef, ast.FunctionDef)):
            names.add(node.name)
        elif isinstance(node, (ast.Import, ast.ImportFrom)):
            names.update(alias.asname or alias.name.split(".", 1)[0] for alias in node.names)
        elif isinstance(node, ast.Assign):
            names.update(target.id for target in node.targets if isinstance(target, ast.Name))
        elif isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name):
            names.add(node.target.id)
    return {name for name in names if not name.startswith("_")}


class DiscordBotTypeExportsTests(unittest.TestCase):
    def test_type_facade_matches_runtime_exports(self) -> None:
        names = _public_stub_names()

        self.assertEqual([name for name in sorted(names) if not hasattr(bot, name)], [])

    def test_representative_exports_keep_runtime_identity(self) -> None:
        self.assertEqual(type(bot.get_runtime_state.__self__).__name__, "RuntimeStateBridge")
        self.assertIs(bot.build_startup_notice, discord_text.build_startup_notice)
        self.assertEqual(bot.discord_stream.__name__, "codex_discord_stream")
        self.assertIsInstance(bot.SOCKET_RUNTIME, socket_runtime.BotSocketRuntime)
        self.assertIsInstance(bot.HISTORY_RUNTIME, history_runtime.BotHistoryRuntime)
        self.assertIsInstance(bot.MESSAGE_RUNTIME, message_runtime.BotMessageRuntime)
        self.assertIsInstance(
            bot.PROCESSED_MESSAGE_RUNTIME,
            processed_message_runtime.ProcessedMessageRuntime,
        )
        self.assertEqual(
            bot.claim_discord_message,
            bot.PROCESSED_MESSAGE_RUNTIME.claim_discord_message,
        )
        self.assertEqual(
            bot.mark_discord_message_processed,
            bot.PROCESSED_MESSAGE_RUNTIME.mark_discord_message_processed,
        )


if __name__ == "__main__":
    _ = unittest.main()
