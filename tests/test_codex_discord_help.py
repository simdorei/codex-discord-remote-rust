from __future__ import annotations

import os
import re
import unittest
from pathlib import Path
from unittest.mock import patch

import codex_discord_bot as bot
import codex_discord_help as discord_help
import codex_discord_runtime_config as runtime_config


EXPECTED_SLASH_COMMANDS = {
    "help",
    "list",
    "archived_list",
    "use",
    "status",
    "settings",
    "doctor",
    "where",
    "context",
    "usage",
    "runners",
    "retract",
    "mirror_check",
    "bridge_sync",
    "new",
    "ask",
    "interview",
    "ask_ipc",
}

REMOVED_COMMAND_TOKENS = (
    "/github_triage",
    "/maintainer_orchestrator",
    "!ask",
    "!triage",
    "!orchestrate",
)


def build_help() -> str:
    return discord_help.build_help(
        qa_commands_enabled=runtime_config.discord_qa_commands_enabled(),
        host_commands_enabled=runtime_config.discord_host_commands_enabled(),
    )


class DiscordHelpContractTests(unittest.TestCase):
    def test_help_readme_and_default_slash_commands_match(self) -> None:
        help_text = build_help()
        help_match = re.search(r"Slash commands: (.+)", help_text)
        self.assertIsNotNone(help_match)
        assert help_match is not None
        help_commands = set(re.findall(r"/([a-z_]+)", help_match.group(1)))
        self.assertEqual(help_commands, EXPECTED_SLASH_COMMANDS)

        readme = Path("README.md").read_text(encoding="utf-8")
        readme_match = re.search(
            r"Registered Discord slash commands:\s*\n\s*-\s*(.+)",
            readme,
        )
        self.assertIsNotNone(readme_match)
        assert readme_match is not None
        readme_commands = set(re.findall(r"/([a-z_]+)", readme_match.group(1)))
        self.assertEqual(readme_commands, EXPECTED_SLASH_COMMANDS)
        help_prefix_commands = set(re.findall(r"^!([a-z_-]+)", help_text, re.MULTILINE))
        readme_prefix_commands = set(re.findall(r"`!([a-z_-]+)", readme))
        self.assertLessEqual(help_prefix_commands, readme_prefix_commands)
        self.assertIn("Numeric refs follow the same DB-root numbering as `!list`.", readme)
        self.assertIn("!mirror check [limit]", help_text)
        self.assertIn("!stop [ref]", help_text)

        source = "\n".join(
            Path(path).read_text(encoding="utf-8")
            for path in [
                bot.__file__,
                "codex_discord_bot_new_thread_adapter_runtime.py",
                "codex_discord_slash_commands.py",
                "codex_discord_slash_prompt_commands.py",
                "codex_discord_slash_runtime_commands.py",
            ]
        )
        command_names = set(re.findall(r'@bot\.tree\.command\(\s*name="([^"]+)"', source))
        self.assertEqual(command_names, EXPECTED_SLASH_COMMANDS | {"qa_buttons"})
        self.assertIn("slash_new_dispatch", source)
        self.assertIn("slash_new_done", source)

        for token in REMOVED_COMMAND_TOKENS:
            self.assertNotIn(token, help_text)

    def test_qa_commands_are_hidden_unless_enabled(self) -> None:
        self.assertNotIn("!qa buttons", build_help())
        self.assertNotIn("!steer", build_help())
        with patch.dict(os.environ, {"DISCORD_ENABLE_QA_COMMANDS": "1"}):
            help_text = build_help()

        self.assertIn("!qa buttons", help_text)
        self.assertIn("!steer <prompt>", help_text)
        help_match = re.search(r"Slash commands: (.+)", help_text)
        self.assertIsNotNone(help_match)
        assert help_match is not None
        self.assertIn("qa_buttons", set(re.findall(r"/([a-z_]+)", help_match.group(1))))
