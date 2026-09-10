# pyright: reportAny=false, reportUnusedCallResult=false
from __future__ import annotations

import unittest

import codex_desktop_bridge as bridge
import codex_desktop_bridge_cli as bridge_cli


class DesktopBridgeCliParserTests(unittest.TestCase):
    def test_public_cli_surface_exports_parser_types_and_repl_helpers(self) -> None:
        self.assertTrue(callable(bridge_cli.build_parser))
        self.assertTrue(callable(bridge_cli.split_repl_command))
        self.assertTrue(callable(bridge_cli.normalize_repl_argv))
        self.assertIn("ask", bridge_cli.REPL_KNOWN_COMMANDS)

        handlers = bridge.make_cli_handlers()

        self.assertIsInstance(handlers, bridge_cli.BridgeCommandHandlers)

    def test_parser_wires_list_settings_options_and_ask_defaults(self) -> None:
        parser = bridge_cli.build_parser(bridge.make_cli_handlers())

        list_args = parser.parse_args(["list", "--limit", "3", "--db-root"])
        self.assertIs(list_args.func, bridge.command_list)
        self.assertEqual(list_args.limit, 3)
        self.assertTrue(list_args.db_root)

        settings_args = parser.parse_args(
            [
                "settings",
                "repo",
                "--model",
                "gpt-5.4",
                "--reasoning",
                "high",
                "--speed",
                "fast",
            ]
        )
        self.assertIs(settings_args.func, bridge.command_settings)
        self.assertEqual(settings_args.thread_ref, "repo")
        self.assertEqual(settings_args.model, "gpt-5.4")
        self.assertEqual(settings_args.reasoning, "high")
        self.assertEqual(settings_args.speed, "fast")

        options_args = parser.parse_args(["settings_options", "--field", "speed"])
        self.assertIs(options_args.func, bridge.command_settings_options)
        self.assertEqual(options_args.field, "speed")

        ask_args = parser.parse_args(["ask", "hello"])
        self.assertIs(ask_args.func, bridge.command_ask)
        self.assertTrue(ask_args.wait)
        self.assertFalse(ask_args.background)
        self.assertTrue(ask_args.ipc)
        self.assertFalse(ask_args.sidecar)
        self.assertFalse(ask_args.ipc_recover_ui)
        self.assertTrue(ask_args.no_fallback)
        self.assertFalse(ask_args.switch_thread)
        self.assertFalse(ask_args.stream)
        self.assertFalse(ask_args.include_commentary)

    def test_parser_wires_desktop_lifecycle_commands(self) -> None:
        parser = bridge_cli.build_parser(bridge.make_cli_handlers())

        discover_args = parser.parse_args(["discover_codex"])
        self.assertIs(discover_args.func, bridge.command_discover_codex)

        restart_args = parser.parse_args(["restart_codex", "--stop-wait", "2.5", "--start-wait", "4"])
        self.assertIs(restart_args.func, bridge.command_restart_codex)
        self.assertEqual(restart_args.stop_wait, 2.5)
        self.assertEqual(restart_args.start_wait, 4.0)

        restart_default_args = parser.parse_args(["restart_codex"])
        self.assertEqual(restart_default_args.stop_wait, 1.0)
        self.assertEqual(restart_default_args.start_wait, 2.0)

        focus_args = parser.parse_args(
            [
                "focus",
                "--thread-id",
                "thread-1",
                "--cwd",
                "C:\\repo",
                "--click",
                "--click-x-ratio",
                "0.25",
                "--click-y-offset",
                "120",
            ]
        )
        self.assertIs(focus_args.func, bridge.command_focus)
        self.assertEqual(focus_args.thread_id, "thread-1")
        self.assertEqual(focus_args.cwd, "C:\\repo")
        self.assertTrue(focus_args.click)
        self.assertEqual(focus_args.click_x_ratio, 0.25)
        self.assertEqual(focus_args.click_y_offset, 120)

        focus_default_args = parser.parse_args(["focus"])
        self.assertFalse(focus_default_args.click)
        self.assertEqual(focus_default_args.click_x_ratio, 0.5)
        self.assertEqual(focus_default_args.click_y_offset, 90)

        new_args = parser.parse_args(
            [
                "new",
                "hello",
                "--abort",
                "--cwd",
                "C:\\repo",
                "--click",
                "--click-x-ratio",
                "0.75",
                "--click-y-offset",
                "88",
                "--create-timeout",
                "12.5",
            ]
        )
        self.assertIs(new_args.func, bridge.command_new)
        self.assertEqual(new_args.prompt, "hello")
        self.assertTrue(new_args.abort)
        self.assertEqual(new_args.cwd, "C:\\repo")
        self.assertTrue(new_args.click)
        self.assertEqual(new_args.click_x_ratio, 0.75)
        self.assertEqual(new_args.click_y_offset, 88)
        self.assertEqual(new_args.create_timeout, 12.5)

        new_default_args = parser.parse_args(["new"])
        self.assertIsNone(new_default_args.prompt)
        self.assertFalse(new_default_args.abort)
        self.assertIsNone(new_default_args.cwd)
        self.assertFalse(new_default_args.click)
        self.assertEqual(new_default_args.create_timeout, 30.0)

    def test_parser_wires_thread_action_commands(self) -> None:
        parser = bridge_cli.build_parser(bridge.make_cli_handlers())

        archive_args = parser.parse_args(["archive", "repo", "--timeout", "3.5", "--no-kill-codex-on-lock"])
        self.assertIs(archive_args.func, bridge.command_archive)
        self.assertEqual(archive_args.thread_ref, "repo")
        self.assertEqual(archive_args.timeout, 3.5)
        self.assertTrue(archive_args.no_kill_codex_on_lock)

        archive_default_args = parser.parse_args(["archive"])
        self.assertIsNone(archive_default_args.thread_ref)
        self.assertEqual(archive_default_args.timeout, 8.0)
        self.assertFalse(archive_default_args.no_kill_codex_on_lock)

        delete_args = parser.parse_args(["delete_archive", "2", "--confirm"])
        self.assertIs(delete_args.func, bridge.command_delete_archive)
        self.assertEqual(delete_args.thread_ref, "2")
        self.assertTrue(delete_args.confirm)

        tail_args = parser.parse_args(["tail", "--thread-id", "thread-1", "--timeout", "6", "--only-new"])
        self.assertIs(tail_args.func, bridge.command_tail)
        self.assertEqual(tail_args.thread_id, "thread-1")
        self.assertEqual(tail_args.timeout, 6.0)
        self.assertTrue(tail_args.only_new)

        tail_default_args = parser.parse_args(["tail"])
        self.assertEqual(tail_default_args.timeout, 0.0)
        self.assertFalse(tail_default_args.only_new)

        open_args = parser.parse_args(["open", "repo", "--cwd", "C:\\repo", "--abort"])
        self.assertIs(open_args.func, bridge.command_open)
        self.assertEqual(open_args.thread_ref, "repo")
        self.assertEqual(open_args.cwd, "C:\\repo")
        self.assertTrue(open_args.abort)

        stop_args = parser.parse_args(["stop", "--thread-id", "thread-1"])
        self.assertIs(stop_args.func, bridge.command_stop)
        self.assertEqual(stop_args.thread_id, "thread-1")
        self.assertIsNone(stop_args.thread_ref)

        stop_ref_args = parser.parse_args(["stop", "repo", "--cwd", "C:\\repo"])
        self.assertIs(stop_ref_args.func, bridge.command_stop)
        self.assertEqual(stop_ref_args.thread_ref, "repo")
        self.assertEqual(stop_ref_args.cwd, "C:\\repo")

        use_args = parser.parse_args(["use", "repo", "--clear"])
        self.assertIs(use_args.func, bridge.command_use)
        self.assertEqual(use_args.thread_ref, "repo")
        self.assertTrue(use_args.clear)

        use_default_args = parser.parse_args(["use"])
        self.assertIsNone(use_default_args.thread_ref)
        self.assertFalse(use_default_args.clear)

        approval_args = parser.parse_args(["approval_reply", "1", "repo", "--timeout", "4"])
        self.assertIs(approval_args.func, bridge.command_approval_reply)
        self.assertEqual(approval_args.answer, "1")
        self.assertEqual(approval_args.thread_ref, "repo")
        self.assertEqual(approval_args.timeout, 4.0)

    def test_repl_dispatch_edges_stay_bounded(self) -> None:
        parser = bridge_cli.build_parser(bridge.make_cli_handlers())
        with self.assertRaises(SystemExit):
            _ = parser.parse_args(["settings_options", "--field", "tier"])

        self.assertEqual(
            bridge_cli.split_repl_command('ask "테스트 문장"'),
            ["ask", "테스트 문장"],
        )

        self.assertEqual(
            bridge_cli.normalize_repl_argv(["그냥", "질문"], original_line="그냥 질문"),
            ["ask", "--stream", "--include-commentary", "그냥 질문"],
        )

        explicit_ask = bridge_cli.normalize_repl_argv(
            ["ask", "--stream", "hello"],
            original_line="ask --stream hello",
        )
        self.assertEqual(explicit_ask.count("--foreground"), 1)
        self.assertEqual(explicit_ask.count("--stream"), 1)
        self.assertEqual(explicit_ask.count("--include-commentary"), 1)


if __name__ == "__main__":
    unittest.main()
