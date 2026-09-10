from __future__ import annotations

import unittest

import codex_discord_commands as commands
from codex_discord_settings_commands import SettingsBridgeAction


class DiscordCommandParserTests(unittest.TestCase):
    def test_list_limit_keeps_user_root_scope(self) -> None:
        argv = commands.build_list_argv("10")

        self.assertEqual(argv, ["list", "--db-root", "--limit", "10"])

    def test_prefix_bridge_action_builds_shared_argv(self) -> None:
        def fake_resolve(channel_id: int | None, ref: str | None) -> list[str]:
            return ["--thread-id", f"{channel_id}:{ref or '-'}"]

        list_default = commands.build_prefix_bridge_action(
            "list",
            "",
            222,
            resolve_target_args_func=fake_resolve,
        )
        list_limited = commands.build_prefix_bridge_action(
            "list",
            "10",
            222,
            resolve_target_args_func=fake_resolve,
        )
        status = commands.build_prefix_bridge_action(
            "status",
            "abc",
            222,
            resolve_target_args_func=fake_resolve,
        )
        open_abort = commands.build_prefix_bridge_action(
            "open_abort",
            "taxlab:1",
            222,
            resolve_target_args_func=fake_resolve,
        )
        stop = commands.build_prefix_bridge_action(
            "stop",
            "",
            222,
            resolve_target_args_func=fake_resolve,
        )
        stop_ref = commands.build_prefix_bridge_action(
            "stop",
            "taxlab:1",
            222,
            resolve_target_args_func=fake_resolve,
        )
        missing_use = commands.build_prefix_bridge_action(
            "use",
            "",
            222,
            resolve_target_args_func=fake_resolve,
        )
        confirm_delete = commands.build_prefix_bridge_action(
            "confirm_delete_archive",
            "thread-1",
            222,
            resolve_target_args_func=fake_resolve,
        )
        archive = commands.build_prefix_bridge_action(
            "archive",
            "4",
            222,
            resolve_target_args_func=fake_resolve,
            resolve_archive_target_args_func=lambda channel_id, ref: [
                "--archive-target",
                f"{channel_id}:{ref or '-'}",
            ],
        )

        self.assertIsNotNone(list_default)
        self.assertIsNotNone(list_limited)
        self.assertIsNotNone(status)
        self.assertIsNotNone(open_abort)
        self.assertIsNotNone(stop)
        self.assertIsNotNone(stop_ref)
        self.assertIsNotNone(missing_use)
        self.assertIsNotNone(confirm_delete)
        self.assertIsNotNone(archive)
        assert list_default is not None
        assert list_limited is not None
        assert status is not None
        assert open_abort is not None
        assert stop is not None
        assert stop_ref is not None
        assert missing_use is not None
        assert confirm_delete is not None
        assert archive is not None
        self.assertEqual(list_default.argv, ["list", "--db-root", "--limit", "0"])
        self.assertEqual(list_limited.argv, ["list", "--db-root", "--limit", "10"])
        self.assertEqual(status.argv, ["status", "--thread-id", "222:abc"])
        self.assertEqual(open_abort.argv, ["open", "--abort", "taxlab:1"])
        self.assertEqual(stop.argv, ["stop", "--thread-id", "222:-"])
        self.assertEqual(stop_ref.argv, ["stop", "--thread-id", "222:taxlab:1"])
        self.assertEqual(missing_use.usage, "Usage: !use <ref>")
        self.assertEqual(confirm_delete.argv, ["delete_archive", "--confirm", "thread-1"])
        self.assertEqual(archive.argv, ["archive", "--archive-target", "222:4"])
        self.assertEqual(commands.parse_usage_days("bad").usage, "Usage: !usage [days]")

    def test_bridge_sync_and_mirror_actions_parse_limits_and_usage(self) -> None:
        self.assertIsNone(commands.parse_bridge_sync_limit("sync", "").limit)
        self.assertIsNone(commands.parse_mirror_action("sync").limit)
        self.assertIsNone(commands.parse_mirror_action("list").limit)
        self.assertEqual(commands.parse_mirror_action("check 7").limit, 7)
        self.assertEqual(
            commands.parse_bridge_sync_limit("bridge", "bad 1").usage,
            "Usage: !bridge sync [limit]",
        )
        self.assertEqual(commands.parse_mirror_action("sync 7").usage, "Usage: !mirror sync")
        self.assertEqual(
            commands.parse_mirror_action("bad").usage,
            "Usage: !mirror sync | !mirror list [limit] | !mirror check [limit]",
        )
        self.assertEqual(commands.parse_mirror_action("doctor").subcommand, "check")

    def test_settings_command_maps_effort_alias_and_speed(self) -> None:
        def fake_resolve(channel_id: int | None, ref: str | None) -> list[str]:
            return ["--target", f"{channel_id}:{ref or '-'}"]

        action = commands.build_prefix_bridge_action(
            "settings",
            "--model gpt-5.5 --effort xhigh --speed standard",
            222,
            resolve_target_args_func=fake_resolve,
        )

        self.assertIsNotNone(action)
        assert action is not None
        self.assertEqual(
            action.argv,
            [
                "settings",
                "--target",
                "222:-",
                "--model",
                "gpt-5.5",
                "--reasoning",
                "xhigh",
                "--speed",
                "standard",
            ],
        )

    def test_settings_command_accepts_explicit_ref(self) -> None:
        def fake_resolve(channel_id: int | None, ref: str | None) -> list[str]:
            return ["--target", f"{channel_id}:{ref or '-'}"]

        action = commands.build_prefix_bridge_action(
            "settings",
            "repo:2 --model gpt-5.4",
            333,
            resolve_target_args_func=fake_resolve,
        )

        self.assertIsNotNone(action)
        assert action is not None
        self.assertEqual(
            action.argv,
            ["settings", "--target", "333:repo:2", "--model", "gpt-5.4"],
        )

    def test_settings_command_with_ref_only_routes_all_choices_to_app_backed_bridge_command(self) -> None:
        def fake_resolve(channel_id: int | None, ref: str | None) -> list[str]:
            return ["--target", f"{channel_id}:{ref or '-'}"]

        action = commands.build_prefix_bridge_action(
            "settings",
            "repo:2",
            333,
            resolve_target_args_func=fake_resolve,
        )

        self.assertIsNotNone(action)
        assert action is not None
        self.assertEqual(action.argv, ["settings_options", "--field", "all"])

    def test_setting_alias_routes_model_choices_to_app_backed_bridge_command(self) -> None:
        def fake_resolve(channel_id: int | None, ref: str | None) -> list[str]:
            return ["--target", f"{channel_id}:{ref or '-'}"]

        action = commands.build_prefix_bridge_action(
            "setting",
            "--model",
            333,
            resolve_target_args_func=fake_resolve,
        )

        self.assertIsNotNone(action)
        assert action is not None
        self.assertEqual(action.argv, ["settings_options", "--field", "model"])

    def test_settings_command_routes_speed_choices_to_app_backed_bridge_command(self) -> None:
        def fake_resolve(channel_id: int | None, ref: str | None) -> list[str]:
            return ["--target", f"{channel_id}:{ref or '-'}"]

        action = commands.build_prefix_bridge_action(
            "settings",
            "--speed",
            333,
            resolve_target_args_func=fake_resolve,
        )

        self.assertIsNotNone(action)
        assert action is not None
        self.assertEqual(action.argv, ["settings_options", "--field", "speed"])

    def test_settings_command_without_args_routes_all_choices_to_app_backed_bridge_command(self) -> None:
        def fake_resolve(channel_id: int | None, ref: str | None) -> list[str]:
            return ["--target", f"{channel_id}:{ref or '-'}"]

        action = commands.build_prefix_bridge_action(
            "settings",
            "",
            333,
            resolve_target_args_func=fake_resolve,
        )

        self.assertIsNotNone(action)
        assert action is not None
        self.assertEqual(action.argv, ["settings_options", "--field", "all"])

    def test_settings_bridge_action_is_slotted_value_object(self) -> None:
        action = SettingsBridgeAction(["settings"], "Settings")

        self.assertEqual(action.argv, ["settings"])
        self.assertFalse(hasattr(action, "__dict__"))
