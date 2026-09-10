# pyright: reportAny=false, reportAttributeAccessIssue=false, reportPrivateUsage=false, reportUnknownArgumentType=false, reportUnknownLambdaType=false, reportUnknownMemberType=false, reportUnusedCallResult=false
from __future__ import annotations

import io
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path

import codex_desktop_bridge as bridge
from codex_pro_prompt_contract import PRO_SKILL_CALL
from tests import codex_desktop_bridge_command_ask_fakes as fakes


class CommandAskHappyTests(unittest.TestCase):
    def test_dry_run_sidecar_ipc_ui_background_and_bridge_wrapper(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            thread = fakes.thread(root, "thread-1")
            other = fakes.thread(root, "other-thread")

            dry_deps = fakes.FakeDeps(thread=thread)
            dry_output, dry_exit = fakes.run_with_output(
                fakes.args(prompt="dry", dry_run=True),
                dry_deps,
            )
            self.assertEqual(dry_exit, 0)
            self.assertIn("[dry_run]\ndry", dry_output)
            self.assertEqual(dry_deps.calls, ["choose:thread-1:None"])

            sidecar_client = fakes.fake_sidecar_client()
            sidecar_deps = fakes.FakeDeps(
                thread=thread,
                sidecar_result={"turn_id": "turn-1", "attempts": "1", "_client": sidecar_client},
                watch_result=fakes.final_result("done"),
            )
            sidecar_output, sidecar_exit = fakes.run_with_output(
                fakes.args(prompt="sidecar", sidecar=True),
                sidecar_deps,
            )
            self.assertEqual(sidecar_exit, 0)
            self.assertTrue(sidecar_client.closed)
            self.assertIn("transport: local-sidecar turn/start", sidecar_output)
            self.assertIn("[delivery_verified] thread-1", sidecar_output)
            self.assertIn("[sidecar_delivery] turn_id=turn-1 attempts=1", sidecar_output)
            self.assertIn("[final_answer]\ndone", sidecar_output)

            ipc_deps = fakes.FakeDeps(
                thread=thread,
                ipc_result={
                    "owner_client_id": "client-1",
                    "turn_id": "turn-2",
                    "recovery_method": "sidebar",
                },
                watch_result=fakes.final_result("ipc done"),
            )
            ipc_output, ipc_exit = fakes.run_with_output(fakes.args(prompt="ipc", ipc=True), ipc_deps)
            self.assertEqual(ipc_exit, 0)
            self.assertIn("ui_activation: ipc-thread-follower-start-turn", ipc_output)
            self.assertIn("[ipc_recovery] sidebar", ipc_output)
            self.assertIn("[ipc_delivery] owner_client=client-1 turn_id=turn-2", ipc_output)

            ui_deps = fakes.FakeDeps(thread=thread, delivery_thread=thread, window=fakes.window_info())
            ui_output, ui_exit = fakes.run_with_output(
                fakes.args(prompt="ui", ipc=False, wait=False),
                ui_deps,
            )
            self.assertEqual(ui_exit, 0)
            self.assertIn("ui_activation: already-open [header]", ui_output)
            self.assertIn("sent_to_window: hwnd=100 title=Codex", ui_output)
            self.assertIn("[delivery_verified] thread-1", ui_output)
            self.assertEqual(ui_deps.delivery_timeouts, [4.0])

            pro_ui_deps = fakes.FakeDeps(thread=thread, delivery_thread=thread, window=fakes.window_info())
            _, pro_ui_exit = fakes.run_with_output(
                fakes.args(prompt=PRO_SKILL_CALL, ipc=False, wait=False),
                pro_ui_deps,
            )
            self.assertEqual(pro_ui_exit, 0)
            self.assertEqual(pro_ui_deps.delivery_timeouts, [15.0])

            background_deps = fakes.FakeDeps(thread=thread, background_started=True)
            background_output, background_exit = fakes.run_with_output(
                fakes.args(prompt="background", ipc=True, background=True),
                background_deps,
            )
            self.assertEqual(background_exit, 0)
            self.assertIn("[background_watch_started] thread-1", background_output)

            already_deps = fakes.FakeDeps(thread=thread, background_started=False)
            already_output, already_exit = fakes.run_with_output(
                fakes.args(prompt="background", ipc=True, background=True),
                already_deps,
            )
            self.assertEqual(already_exit, 0)
            self.assertIn("[background_watch_already_running] thread-1", already_output)

            original_deps = bridge._make_command_ask_deps
            try:
                bridge._make_command_ask_deps = lambda: fakes.FakeDeps(thread=other).as_command_deps()
                parser = bridge.build_parser()
                parsed = parser.parse_args(["ask", "--thread-id", "other-thread", "--dry-run", "hello"])
                self.assertIs(parsed.func, bridge.command_ask)
                wrapper_output = io.StringIO()
                with redirect_stdout(wrapper_output):
                    wrapper_exit = parsed.func(parsed)
            finally:
                bridge._make_command_ask_deps = original_deps
            self.assertEqual(wrapper_exit, 0)
            self.assertIn("target_thread: other-thread", wrapper_output.getvalue())


if __name__ == "__main__":
    unittest.main()
