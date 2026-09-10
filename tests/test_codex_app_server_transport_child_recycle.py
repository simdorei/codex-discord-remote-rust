from __future__ import annotations

import json
import subprocess
import threading
import unittest
from typing import cast
from unittest import mock

import codex_app_server_transport as transport_mod
from codex_app_server_transport_goal import GoalAbsent
from codex_app_server_transport_lifecycle import ChildCleanupRecycleStatus
from codex_app_server_transport_replies import JsonMapping, JsonObject
from codex_app_server_transport_subscriptions import ThreadReleaseStatus


class _RunningProcess:
    def poll(self) -> None:
        return None


def _install_generation(client: transport_mod.PersistentCodexAppServer, generation: int) -> None:
    client.process = cast(subprocess.Popen[str], cast(object, _RunningProcess()))
    client._initialized = True
    client._generation = generation
    client._accepting_since = 100.0
    client._children.reset(generation)


def _notification(method: str, params: JsonMapping) -> str:
    return json.dumps({"method": method, "params": dict(params)})


class AppServerChildRecycleTests(unittest.TestCase):
    def test_child_debt_recycles_once_only_after_all_subscribed_roots_are_terminal(self) -> None:
        client = transport_mod.PersistentCodexAppServer(executable_resolver=lambda: "codex.exe")
        _install_generation(client, 7)
        client.mark_thread_subscribed("root-a")
        client.mark_thread_subscribed("root-b")
        client._handle_raw_line(
            _notification(
                "thread/started",
                {
                    "thread": {
                        "id": "child-a",
                        "parentThreadId": "root-a",
                        "source": {"subAgent": {"thread_spawn": {"parent_thread_id": "root-a"}}},
                    }
                },
            )
        )
        client._handle_raw_line(
            _notification(
                "item/completed",
                {
                    "threadId": "root-a",
                    "turnId": "turn-a",
                    "item": {
                        "type": "collabAgentToolCall",
                        "id": "spawn-a",
                        "tool": "spawnAgent",
                        "status": "completed",
                        "senderThreadId": "root-a",
                        "receiverThreadIds": ["child-a"],
                    },
                },
            )
        )
        client._handle_raw_line(
            _notification(
                "turn/started",
                {
                    "threadId": "root-b",
                    "turn": {"id": "turn-b", "status": "inProgress"},
                },
            )
        )
        restarts: list[bool] = []

        def has_active_turn(
            thread_id: str,
            *,
            expected_generation: int | None = None,
        ) -> bool:
            self.assertEqual(expected_generation, 7)
            return client._pending.active_turn_id(thread_id) is not None

        def request(
            method: str,
            params: JsonMapping | None = None,
            *,
            timeout_sec: float = 10.0,
            expected_generation: int | None = None,
        ) -> JsonObject:
            self.assertEqual(
                (method, dict(params or {}), timeout_sec, expected_generation),
                (
                    "thread/unsubscribe",
                    {"threadId": str((params or {}).get("threadId") or "")},
                    8.0,
                    7,
                ),
            )
            return {}

        def restart() -> None:
            restarts.append(True)
            client._generation = 8
            client._children.reset(8)

        with (
            mock.patch.object(client, "has_active_turn_or_raise", has_active_turn),
            mock.patch.object(client, "get_thread_goal_lookup", return_value=GoalAbsent()),
            mock.patch.object(client, "request", request),
            mock.patch.object(client, "restart", side_effect=restart),
        ):
            released_a = client.release_thread_subscription_if_terminal(
                "root-a",
                expected_generation=7,
            )
            self.assertEqual(released_a.status, ThreadReleaseStatus.RELEASED)
            self.assertEqual(restarts, [])

            client._handle_raw_line(
                _notification(
                    "turn/completed",
                    {
                        "threadId": "root-b",
                        "turn": {"id": "turn-b", "status": "completed"},
                    },
                )
            )
            released_b = client.release_thread_subscription_if_terminal(
                "root-b",
                expected_generation=7,
            )

        self.assertEqual(released_b.status, ThreadReleaseStatus.RELEASED)
        self.assertEqual(restarts, [True])

    def test_terminal_release_without_a_spawned_child_does_not_recycle(self) -> None:
        client = transport_mod.PersistentCodexAppServer(executable_resolver=lambda: "codex.exe")
        _install_generation(client, 4)
        client.mark_thread_subscribed("root")

        with (
            mock.patch.object(client, "has_active_turn_or_raise", return_value=False),
            mock.patch.object(client, "get_thread_goal_lookup", return_value=GoalAbsent()),
            mock.patch.object(client, "request", return_value={}),
            mock.patch.object(client, "restart") as restart,
        ):
            released = client.release_thread_subscription_if_terminal(
                "root",
                expected_generation=4,
            )

        self.assertEqual(released.status, ThreadReleaseStatus.RELEASED)
        restart.assert_not_called()

    def test_external_queue_work_defers_recycle_without_losing_cleanup_debt(self) -> None:
        client = transport_mod.PersistentCodexAppServer(executable_resolver=lambda: "codex.exe")
        _install_generation(client, 9)
        client.mark_thread_subscribed("root")
        client._handle_raw_line(
            _notification(
                "item/completed",
                {
                    "threadId": "root",
                    "turnId": "turn",
                    "item": {
                        "type": "collabAgentToolCall",
                        "id": "spawn",
                        "tool": "spawn_agent",
                        "status": "completed",
                        "senderThreadId": "root",
                        "receiverThreadIds": ["child"],
                    },
                },
            )
        )
        queue_busy = True
        client._external_work_guard = lambda: queue_busy
        restarts: list[bool] = []

        def restart() -> None:
            restarts.append(True)
            client._generation = 10
            client._children.reset(10)

        with (
            mock.patch.object(client, "has_active_turn_or_raise", return_value=False),
            mock.patch.object(client, "get_thread_goal_lookup", return_value=GoalAbsent()),
            mock.patch.object(client, "request", return_value={}),
            mock.patch.object(client, "restart", side_effect=restart),
        ):
            first = client.release_thread_subscription_if_terminal(
                "root",
                expected_generation=9,
            )
            self.assertEqual(first.status, ThreadReleaseStatus.RELEASED)
            self.assertEqual(restarts, [])

            queue_busy = False
            second = client.release_thread_subscription_if_terminal(
                "root",
                expected_generation=9,
            )

        self.assertEqual(second.status, ThreadReleaseStatus.ALREADY_RELEASED)
        self.assertEqual(restarts, [True])

    def test_delivery_admission_blocks_recycle_until_delivery_exits(self) -> None:
        client = transport_mod.PersistentCodexAppServer(executable_resolver=lambda: "codex.exe")
        _install_generation(client, 11)
        client._handle_raw_line(
            _notification(
                "item/completed",
                {
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
            )
        )
        restarts: list[bool] = []
        restarted = threading.Event()

        def restart() -> None:
            restarts.append(True)
            client._generation = 12
            client._children.reset(12)
            restarted.set()

        with mock.patch.object(client, "restart", side_effect=restart):
            with client.delivery_admission(11) as snapshot:
                blocked = client.try_recycle_child_cleanup(expected_generation=11)
                self.assertEqual(snapshot.generation, 11)
                self.assertEqual(blocked.status, ChildCleanupRecycleStatus.ACTIVE_DELIVERY)
                self.assertEqual(restarts, [])
            self.assertTrue(restarted.wait(timeout=2.0))

        self.assertEqual(restarts, [True])

    def test_deferred_external_work_retries_when_the_blocker_clears(self) -> None:
        client = transport_mod.PersistentCodexAppServer(executable_resolver=lambda: "codex.exe")
        _install_generation(client, 12)
        client.mark_thread_subscribed("root")
        client._handle_raw_line(
            _notification(
                "thread/started",
                {"thread": {"id": "child", "parentThreadId": "root"}},
            )
        )
        queue_busy = True
        client._external_work_guard = lambda: queue_busy
        restarted = threading.Event()

        def restart() -> None:
            client._generation = 13
            client._children.reset(13)
            restarted.set()

        with (
            mock.patch.object(client, "has_active_turn_or_raise", return_value=False),
            mock.patch.object(client, "get_thread_goal_lookup", return_value=GoalAbsent()),
            mock.patch.object(client, "request", return_value={}),
            mock.patch.object(client, "restart", side_effect=restart),
        ):
            released = client.release_thread_subscription_if_terminal(
                "root",
                expected_generation=12,
            )
            self.assertEqual(released.status, ThreadReleaseStatus.RELEASED)
            self.assertFalse(restarted.is_set())

            queue_busy = False
            client.notify_child_cleanup_blocker_changed()

            self.assertTrue(restarted.wait(timeout=2.0))

    def test_simultaneous_recycle_attempts_coalesce_to_one_restart(self) -> None:
        client = transport_mod.PersistentCodexAppServer(executable_resolver=lambda: "codex.exe")
        _install_generation(client, 13)
        client._handle_raw_line(
            _notification(
                "thread/started",
                {"thread": {"id": "child", "parentThreadId": "root"}},
            )
        )
        restart_entered = threading.Event()
        allow_restart = threading.Event()
        outcomes: list[ChildCleanupRecycleStatus] = []

        def restart() -> None:
            restart_entered.set()
            self.assertTrue(allow_restart.wait(timeout=2.0))
            client._generation = 14
            client._children.reset(14)

        def first_recycle() -> None:
            outcome = client.try_recycle_child_cleanup(expected_generation=13)
            outcomes.append(outcome.status)

        with mock.patch.object(client, "restart", side_effect=restart):
            worker = threading.Thread(target=first_recycle)
            worker.start()
            self.assertTrue(restart_entered.wait(timeout=2.0))

            second = client.try_recycle_child_cleanup(expected_generation=13)
            allow_restart.set()
            worker.join(timeout=2.0)

        self.assertFalse(worker.is_alive())
        self.assertEqual(second.status, ChildCleanupRecycleStatus.RECYCLE_BUSY)
        self.assertEqual(outcomes, [ChildCleanupRecycleStatus.RECYCLED])

    def test_failed_owned_tree_close_is_retried_for_the_same_unhealthy_generation(self) -> None:
        client = transport_mod.PersistentCodexAppServer(executable_resolver=lambda: "codex.exe")
        _install_generation(client, 14)
        client._handle_raw_line(
            _notification(
                "thread/started",
                {"thread": {"id": "child", "parentThreadId": "root"}},
            )
        )
        close_attempts = 0
        retried = threading.Event()

        def close_process(_process: object, _log: object) -> None:
            nonlocal close_attempts
            close_attempts += 1
            if close_attempts == 1:
                raise OSError("transient job close failure")
            retried.set()

        with (
            mock.patch(
                "codex_app_server_transport_resident.close_resident_app_server_process",
                side_effect=close_process,
            ),
            mock.patch.object(client, "start", return_value=None),
        ):
            with self.assertRaisesRegex(OSError, "transient job close failure"):
                _ = client.try_recycle_child_cleanup(expected_generation=14)

            self.assertTrue(retried.wait(timeout=2.0))

        self.assertEqual(close_attempts, 2)
        self.assertIsNone(client.process)
        self.assertFalse(client.child_lifecycle_snapshot().cleanup_pending)

    def test_explicit_close_waits_for_inflight_recycle_and_closes_restarted_process(self) -> None:
        client = transport_mod.PersistentCodexAppServer(executable_resolver=lambda: "codex.exe")
        _install_generation(client, 15)
        client._handle_raw_line(
            _notification(
                "thread/started",
                {"thread": {"id": "child", "parentThreadId": "root"}},
            )
        )
        restart_entered = threading.Event()
        allow_restart = threading.Event()
        close_completed = threading.Event()

        def restart() -> None:
            restart_entered.set()
            self.assertTrue(allow_restart.wait(timeout=2.0))
            _install_generation(client, 16)

        def close() -> None:
            client.close()
            close_completed.set()

        with (
            mock.patch.object(client, "restart", side_effect=restart),
            mock.patch(
                "codex_app_server_transport_resident.close_resident_app_server_process"
            ) as close_process,
        ):
            recycle_worker = threading.Thread(
                target=lambda: client.try_recycle_child_cleanup(expected_generation=15)
            )
            recycle_worker.start()
            self.assertTrue(restart_entered.wait(timeout=2.0))

            close_worker = threading.Thread(target=close)
            close_worker.start()
            self.assertFalse(close_completed.wait(timeout=0.1))

            allow_restart.set()
            recycle_worker.join(timeout=2.0)
            close_worker.join(timeout=2.0)

        self.assertFalse(recycle_worker.is_alive())
        self.assertFalse(close_worker.is_alive())
        self.assertTrue(close_completed.is_set())
        close_process.assert_called_once()
        self.assertIsNone(client.process)
        self.assertFalse(client.lifecycle_snapshot().healthy)


if __name__ == "__main__":
    _ = unittest.main()
