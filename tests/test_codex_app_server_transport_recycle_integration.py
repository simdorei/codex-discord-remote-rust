from __future__ import annotations

import json
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest import mock

import codex_app_server_transport as transport_mod
import codex_app_server_transport_process as process_mod
from codex_app_server_transport_goal import GoalAbsent
import codex_app_server_transport_resident as resident_mod
from codex_app_server_transport_subscriptions import ThreadReleaseStatus
from tests.test_codex_app_server_transport_process_tree import (
    _kill_exact_tree,
    _wait_pid_gone,
)


def _fake_server_code(pid_path: Path) -> str:
    grandchild_code = "import time; time.sleep(300)"
    child_code = "\n".join(
        (
            "import json, os, subprocess, sys, time",
            f"grandchild = subprocess.Popen([sys.executable, '-c', {grandchild_code!r}])",
            "print(json.dumps({'child': os.getpid(), 'grandchild': grandchild.pid}), flush=True)",
            "time.sleep(300)",
        )
    )
    return "\n".join(
        (
            "import json, os, pathlib, subprocess, sys, time",
            (
                "child = subprocess.Popen("
                f"[sys.executable, '-u', '-c', {child_code!r}], "
                "stdout=subprocess.PIPE, text=True)"
            ),
            "child_info = json.loads(child.stdout.readline())",
            (
                f"pathlib.Path({str(pid_path)!r}).write_text("
                "json.dumps({'parent': os.getpid(), **child_info}), encoding='utf-8')"
            ),
            "for line in sys.stdin:",
            "    message = json.loads(line)",
            "    request_id = message.get('id')",
            "    if not request_id:",
            "        continue",
            "    method = message.get('method')",
            "    if method == 'thread/list':",
            "        result = {'data': [], 'nextCursor': None}",
            "    else:",
            "        result = {}",
            "    print(json.dumps({'id': request_id, 'result': result}), flush=True)",
            "time.sleep(300)",
        )
    )


class AppServerRecycleIntegrationTests(unittest.TestCase):
    def test_idle_child_debt_recycles_real_process_tree_and_new_generation_responds(self) -> None:
        logs: list[str] = []
        processes: list[process_mod.ResidentProcess] = []
        pid_paths: list[Path] = []
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            root = Path(temp_dir)

            def spawn(_executable: str) -> process_mod.ResidentProcess:
                pid_path = root / f"generation-{len(pid_paths) + 1}.json"
                pid_paths.append(pid_path)
                process = process_mod.start_owned_app_server_command(
                    [sys.executable, "-u", "-c", _fake_server_code(pid_path)]
                )
                processes.append(process)
                return process

            client = transport_mod.PersistentCodexAppServer(
                executable_resolver=lambda: "fake-codex.exe",
                log_func=logs.append,
                generation_seed_func=lambda: 1,
            )
            try:
                with mock.patch.object(
                    resident_mod,
                    "start_resident_app_server_process",
                    side_effect=spawn,
                ):
                    client.start()
                    first_pids = self._read_pids(pid_paths[0])
                    client.mark_thread_subscribed("root")
                    client._handle_raw_line(
                        json.dumps(
                            {
                                "method": "item/completed",
                                "params": {
                                    "threadId": "root",
                                    "item": {
                                        "type": "collabAgentToolCall",
                                        "id": "spawn",
                                        "tool": "spawnAgent",
                                        "status": "completed",
                                        "senderThreadId": "root",
                                        "receiverThreadIds": ["child"],
                                    },
                                },
                            }
                        )
                    )

                    with (
                        mock.patch.object(client, "has_active_turn_or_raise", return_value=False),
                        mock.patch.object(client, "get_thread_goal_lookup", return_value=GoalAbsent()),
                    ):
                        outcome = client.release_thread_subscription_if_terminal(
                            "root",
                            expected_generation=1,
                        )

                    self.assertEqual(outcome.status, ThreadReleaseStatus.RELEASED)
                    self.assertEqual(client.lifecycle_snapshot().generation, 2)
                    self.assertTrue(all(_wait_pid_gone(pid) for pid in first_pids[1:]))
                    response = client.request("thread/list", {}, expected_generation=2)
                    self.assertEqual(response, {"data": [], "nextCursor": None})
                    second_pids = self._read_pids(pid_paths[1])
            finally:
                client.close()
                for process in processes:
                    for pipe in (process.stdin, process.stdout, process.stderr):
                        if pipe is not None and not pipe.closed:
                            pipe.close()
                for pid_path in pid_paths:
                    if not pid_path.exists():
                        continue
                    for pid in self._read_pids(pid_path):
                        _kill_exact_tree(pid)

            self.assertTrue(all(_wait_pid_gone(pid) for pid in second_pids[1:]))
            self.assertTrue(any("app_server_child_cleanup_recycled" in line for line in logs))

    @staticmethod
    def _read_pids(path: Path) -> tuple[int, int, int]:
        deadline = time.monotonic() + 5.0
        while time.monotonic() < deadline and not path.exists():
            time.sleep(0.05)
        payload = json.loads(path.read_text(encoding="utf-8"))
        return int(payload["parent"]), int(payload["child"]), int(payload["grandchild"])


if __name__ == "__main__":
    _ = unittest.main()
