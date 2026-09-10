# pyright: reportAny=false, reportAttributeAccessIssue=false, reportPrivateUsage=false, reportUnknownArgumentType=false, reportUnknownLambdaType=false, reportUnknownMemberType=false, reportUnusedCallResult=false
from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

import codex_desktop_bridge_command_ask as command_ask
import codex_desktop_bridge_command_ask_delivery as command_ask_delivery
from codex_desktop_bridge_final_answer_types import WatchForFinalAnswerResult
from tests import codex_desktop_bridge_command_ask_fakes as fakes


class CommandAskEdgeTests(unittest.TestCase):
    def test_missing_busy_delivery_wait_and_sidecar_close_edges(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            thread = fakes.thread(root, "thread-1")
            other = fakes.thread(root, "other-thread")

            with self.assertRaisesRegex(command_ask.CommandAskError, "Session file not found"):
                command_ask.run_command_ask(
                    fakes.args(),
                    deps=fakes.FakeDeps(thread=fakes.missing_thread(root)).as_command_deps(),
                )

            with self.assertRaisesRegex(RuntimeError, "busy detail"):
                command_ask.run_command_ask(
                    fakes.args(sidecar=True),
                    deps=fakes.FakeDeps(thread=thread, busy_state="busy").as_command_deps(),
                )

            ipc_busy_output, ipc_busy_exit = fakes.run_with_output(
                fakes.args(ipc=True, wait=False),
                fakes.FakeDeps(thread=thread, busy_state="busy"),
            )
            self.assertEqual(ipc_busy_exit, 0)
            self.assertIn("[ipc_delivery]", ipc_busy_output)

            forced_output, forced_exit = fakes.run_with_output(
                fakes.args(sidecar=True, force_while_busy=True, wait=False),
                fakes.FakeDeps(thread=thread, busy_state="busy"),
            )
            self.assertEqual(forced_exit, 0)
            self.assertIn("[sidecar_delivery]", forced_output)

            with self.assertRaisesRegex(command_ask_delivery.CommandAskDeliveryError, "different thread after sidecar"):
                command_ask.run_command_ask(
                    fakes.args(sidecar=True),
                    deps=fakes.FakeDeps(thread=thread, delivery_thread=other).as_command_deps(),
                )

            with self.assertRaisesRegex(command_ask_delivery.CommandAskDeliveryError, "different thread after IPC"):
                command_ask.run_command_ask(
                    fakes.args(ipc=True),
                    deps=fakes.FakeDeps(thread=thread, delivery_thread=other).as_command_deps(),
                )

            with self.assertRaisesRegex(command_ask_delivery.CommandAskDeliveryError, "different thread"):
                command_ask.run_command_ask(
                    fakes.args(ipc=False),
                    deps=fakes.FakeDeps(thread=thread, delivery_thread=other).as_command_deps(),
                )

            with self.assertRaisesRegex(command_ask_delivery.CommandAskDeliveryError, "could not be confirmed"):
                command_ask.run_command_ask(
                    fakes.args(ipc=False),
                    deps=fakes.FakeDeps(thread=thread, delivery_missing=True).as_command_deps(),
                )

            pending_output, pending_exit = fakes.run_with_output(
                fakes.args(sidecar=True, wait=False),
                fakes.FakeDeps(thread=thread, delivery_missing=True),
            )
            self.assertEqual(pending_exit, 0)
            self.assertIn("[delivery_pending]", pending_output)

            cancelled_output, cancelled_exit = fakes.run_with_output(
                fakes.args(ipc=True),
                fakes.FakeDeps(thread=thread, watch_interrupt=True),
            )
            self.assertEqual(cancelled_exit, 0)
            self.assertIn("[wait_cancelled]", cancelled_output)

            timeout_result: WatchForFinalAnswerResult = {
                "status": "timeout",
                "commentary": ["still running"],
                "final_answer": "",
                "streamed_live": False,
                "final_streamed_live": False,
            }
            timeout_output, timeout_exit = fakes.run_with_output(
                fakes.args(ipc=True),
                fakes.FakeDeps(
                    thread=thread,
                    watch_result=timeout_result,
                ),
            )
            self.assertEqual(timeout_exit, 2)
            self.assertIn("[timeout]\nstill running", timeout_output)

            aborted_result: WatchForFinalAnswerResult = {
                "status": "aborted",
                "commentary": [],
                "final_answer": "",
                "streamed_live": False,
                "final_streamed_live": False,
            }
            aborted_output, aborted_exit = fakes.run_with_output(
                fakes.args(ipc=True),
                fakes.FakeDeps(thread=thread, watch_result=aborted_result),
            )
            self.assertEqual(aborted_exit, 0)
            self.assertIn("[aborted]", aborted_output)

            progress_result: WatchForFinalAnswerResult = {
                "status": "progress",
                "commentary": ["more work remains"],
                "final_answer": "",
                "streamed_live": False,
                "final_streamed_live": False,
            }
            progress_output, progress_exit = fakes.run_with_output(
                fakes.args(ipc=True, include_commentary=False),
                fakes.FakeDeps(thread=thread, watch_result=progress_result),
            )
            self.assertEqual(progress_exit, 0)
            self.assertIn("[commentary]\nmore work remains", progress_output)
            self.assertIn("[ready]", progress_output)
            self.assertNotIn("[timeout]", progress_output)

            streamed_result = fakes.final_result("done")
            streamed_result["final_streamed_live"] = True
            streamed_output, streamed_exit = fakes.run_with_output(
                fakes.args(ipc=True),
                fakes.FakeDeps(
                    thread=thread,
                    watch_result=streamed_result,
                ),
            )
            self.assertEqual(streamed_exit, 0)
            self.assertIn("[ready]", streamed_output)


if __name__ == "__main__":
    unittest.main()
