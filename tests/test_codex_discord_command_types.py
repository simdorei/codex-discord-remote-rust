from __future__ import annotations

from dataclasses import FrozenInstanceError
import unittest

import codex_discord_command_types as command_types
import codex_discord_commands as commands


class DiscordCommandTypeTests(unittest.TestCase):
    def test_command_types_are_frozen_and_slotted(self) -> None:
        prefix = command_types.PrefixCommand(command="list", arg="")
        bridge = command_types.PrefixBridgeAction(argv=["list"], title="List")
        limit = command_types.PrefixLimitAction(limit=7)
        mirror = command_types.MirrorAction(subcommand="check", limit=3)

        self.assertEqual(prefix.command, "list")
        self.assertEqual(bridge.argv, ["list"])
        self.assertEqual(limit.limit, 7)
        self.assertEqual(mirror.subcommand, "check")
        self.assertFalse(hasattr(prefix, "__dict__"))
        self.assertFalse(hasattr(bridge, "__dict__"))
        self.assertFalse(hasattr(limit, "__dict__"))
        self.assertFalse(hasattr(mirror, "__dict__"))
        with self.assertRaises(FrozenInstanceError):
            setattr(prefix, "command", "status")

    def test_commands_module_reexports_command_types(self) -> None:
        self.assertIs(commands.PrefixCommand, command_types.PrefixCommand)
        self.assertIs(commands.PrefixBridgeAction, command_types.PrefixBridgeAction)
        self.assertIs(commands.PrefixLimitAction, command_types.PrefixLimitAction)
        self.assertIs(commands.MirrorAction, command_types.MirrorAction)

    def test_raw_command_value_covers_current_parser_inputs(self) -> None:
        values: tuple[command_types.RawCommandValue, ...] = ("", "10", 4, None)

        self.assertEqual(values, ("", "10", 4, None))


if __name__ == "__main__":
    _ = unittest.main()
