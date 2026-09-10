from __future__ import annotations

import io
import json
import subprocess
import threading
import time
import unittest
from collections.abc import Callable
from typing import IO, Final, cast, final, override
from unittest import mock

import codex_app_server_transport_process as process_mod
import codex_app_server_transport_resident as resident_mod
import codex_app_server_transport as transport_mod
from codex_app_server_transport_replies import (
    CodexAppServerTransportError,
    JsonMapping,
    JsonObject,
)
from codex_app_server_transport_resident import ResidentCodexAppServerTransport
from codex_app_server_transport_turn_outcomes import TurnCompletionPending
from codex_plugin_runtime_fingerprint import PluginRuntimeFingerprintError


class ResidentAppServerProcessHelperTests(unittest.TestCase):
    def test_process_helper_preserves_popen_arguments(self) -> None:
        process = _process()
        popen_calls: list[tuple[list[str], dict[str, object]]] = []

        def popen(args: list[str], **kwargs: object) -> subprocess.Popen[str]:
            popen_calls.append((args, kwargs))
            return process

        with (
            mock.patch.object(subprocess, "Popen", popen),
            mock.patch.object(
                process_mod,
                "create_kill_on_close_job_for_suspended_process",
                return_value=None,
            ),
        ):
            started = process_mod.start_resident_app_server_process("codex.exe")

        self.assertEqual(started.pid, process.pid)
        self.assertEqual(len(popen_calls), 1)
        args, kwargs = popen_calls[0]
        self.assertEqual(args, ["codex.exe", "app-server"])
        self.assertEqual(kwargs["stdin"], subprocess.PIPE)
        self.assertEqual(kwargs["stdout"], subprocess.PIPE)
        self.assertEqual(kwargs["stderr"], subprocess.PIPE)
        self.assertTrue(kwargs["text"])
        self.assertEqual(kwargs["encoding"], "utf-8")
        self.assertEqual(kwargs["errors"], "replace")
        self.assertEqual(kwargs["bufsize"], 1)
        self.assertEqual(
            kwargs["creationflags"],
            getattr(subprocess, "CREATE_NO_WINDOW", 0)
            | process_mod.WINDOWS_CREATE_SUSPENDED,
        )

    def test_process_helper_wraps_oserror_with_executable_detail(self) -> None:
        def popen(*_args: object, **_kwargs: object) -> subprocess.Popen[str]:
            raise OSError("missing")

        with mock.patch.object(subprocess, "Popen", popen):
            with self.assertRaisesRegex(
                CodexAppServerTransportError,
                "Failed to start resident Codex app-server. executable='missing.exe'",
            ):
                _ = process_mod.start_resident_app_server_process("missing.exe")

    def test_stdio_helper_detects_missing_pipes(self) -> None:
        self.assertTrue(process_mod.has_resident_app_server_stdio(_process()))
        self.assertFalse(process_mod.has_resident_app_server_stdio(_process(stdin=None)))
        self.assertFalse(process_mod.has_resident_app_server_stdio(_process(stdout=None)))

    def test_close_helper_closes_stdin_and_allows_graceful_exit(self) -> None:
        process = _FakeProcess()
        logs: list[str] = []

        process_mod.close_resident_app_server_process(_as_popen(process), logs.append)

        self.assertTrue(cast(io.StringIO, process.stdin).closed)
        self.assertFalse(process.terminated)
        self.assertEqual(process.wait_timeout, 1.5)
        self.assertFalse(process.killed)
        self.assertEqual(logs, [])

    def test_close_helper_skips_terminate_for_exited_process(self) -> None:
        process = _FakeProcess(poll_result=0)

        process_mod.close_resident_app_server_process(_as_popen(process), lambda _text: None)

        self.assertTrue(cast(io.StringIO, process.stdin).closed)
        self.assertFalse(process.terminated)
        self.assertIsNone(process.wait_timeout)
        self.assertFalse(process.killed)

    def test_close_helper_logs_unowned_terminate_and_kill_failures(self) -> None:
        process = _FakeProcess(
            stdin=_FailingCloseStdin(),
            wait_error=RuntimeError("wait boom"),
            kill_error=RuntimeError("kill boom"),
        )
        logs: list[str] = []

        process_mod.close_resident_app_server_process(_as_popen(process), logs.append)

        self.assertEqual(
            logs,
            [
                "app_server_transport_stdin_close_failed error_type=OSError error=close boom",
                "app_server_transport_graceful_wait_failed error_type=RuntimeError error=wait boom",
                "app_server_transport_graceful_close_timed_out pid=4242",
                "app_server_transport_terminate_failed error_type=RuntimeError error=wait boom",
                "app_server_transport_kill_failed error_type=RuntimeError error=kill boom",
            ],
        )


class ResidentTransportStartTests(unittest.TestCase):
    def test_lifecycle_snapshot_is_passive_before_start(self) -> None:
        resolver_calls: list[bool] = []
        transport = ResidentCodexAppServerTransport(
            executable_resolver=lambda: resolver_calls.append(True) or "codex.exe"
        )

        snapshot = transport.lifecycle_snapshot()

        self.assertEqual(snapshot.generation, 0)
        self.assertFalse(snapshot.healthy)
        self.assertIsNone(snapshot.accepting_since)
        self.assertIsNone(snapshot.plugin_runtime_fingerprint)
        self.assertEqual(resolver_calls, [])

    def test_start_preserves_state_reset_handshake_thread_and_log(self) -> None:
        logs: list[str] = []
        process = _process()
        transport = _StartProbeTransport(executable="codex.exe", log_func=logs.append)
        transport.seed_stale_state(logs.append)

        with (
            mock.patch.object(resident_mod, "start_resident_app_server_process", return_value=process),
            mock.patch.object(threading, "Thread", _FakeThread),
        ):
            transport.start()

        self.assertIs(transport.process, process)
        self.assertEqual(transport.responses_snapshot(), {})
        self.assertEqual(transport.get_pending_server_requests(), [])
        self.assertEqual(
            transport.requests,
            [
                (
                    "initialize",
                    {
                        "clientInfo": {
                            "name": "codex-discord-remote",
                            "title": "Codex Discord Remote",
                            "version": "1.0",
                        },
                        "capabilities": {"experimentalApi": True},
                    },
                    8.0,
                )
            ],
        )
        self.assertEqual(transport.notifications, [("initialized", {})])
        self.assertTrue(transport.initialized)
        self.assertTrue(transport.stdout_thread_started)
        self.assertIn("app_server_transport_started executable=codex.exe", logs)

    def test_successful_initialize_publishes_healthy_generation_snapshot(self) -> None:
        process = _process()
        transport = _StartProbeTransport(executable="codex.exe", wall_time_func=lambda: 1_234.5)

        with (
            mock.patch.object(resident_mod, "start_resident_app_server_process", return_value=process),
            mock.patch.object(threading, "Thread", _FakeThread),
        ):
            transport.start()

        snapshot = transport.lifecycle_snapshot()
        self.assertEqual(snapshot.generation, 1)
        self.assertTrue(snapshot.healthy)
        self.assertEqual(snapshot.accepting_since, 1_234.5)
        self.assertEqual(snapshot.plugin_runtime_fingerprint, "test-fingerprint")
        self.assertIsNone(snapshot.plugin_runtime_error)

        transport.close()
        closed_snapshot = transport.lifecycle_snapshot()
        self.assertEqual(closed_snapshot.generation, 1)
        self.assertFalse(closed_snapshot.healthy)
        self.assertIsNone(closed_snapshot.accepting_since)

    def test_fingerprint_failure_is_preserved_for_fail_closed_pro_preflight(
        self,
    ) -> None:
        def fail_fingerprint() -> str:
            raise PluginRuntimeFingerprintError("inventory query failed")

        logs: list[str] = []
        process = _process()
        transport = _StartProbeTransport(
            executable="codex.exe",
            log_func=logs.append,
            plugin_runtime_fingerprint_reader=fail_fingerprint,
        )
        with (
            mock.patch.object(
                resident_mod, "start_resident_app_server_process", return_value=process
            ),
            mock.patch.object(threading, "Thread", _FakeThread),
        ):
            transport.start()

        snapshot = transport.lifecycle_snapshot()
        self.assertTrue(snapshot.healthy)
        self.assertIsNone(snapshot.plugin_runtime_fingerprint)
        self.assertEqual(snapshot.plugin_runtime_error, "inventory query failed")
        self.assertTrue(
            any("plugin_runtime_fingerprint_failed" in line for line in logs)
        )

    def test_reader_exit_marks_current_generation_unhealthy(self) -> None:
        process = _process()
        transport = _StartProbeTransport(executable="codex.exe", wall_time_func=lambda: 1_234.5)

        with (
            mock.patch.object(resident_mod, "start_resident_app_server_process", return_value=process),
            mock.patch.object(threading, "Thread", _FakeThread),
        ):
            transport.start()

        transport.drain_process(process)

        snapshot = transport.lifecycle_snapshot()
        self.assertEqual(snapshot.generation, 1)
        self.assertFalse(snapshot.healthy)
        self.assertIsNone(snapshot.accepting_since)

    def test_start_closes_process_when_stdio_is_unavailable(self) -> None:
        process = _FakeProcess(stdin=None, stdout=io.StringIO())
        transport = _StartProbeTransport(executable="codex.exe")

        with mock.patch.object(
            resident_mod,
            "start_resident_app_server_process",
            return_value=_as_popen(process),
        ):
            with self.assertRaisesRegex(
                CodexAppServerTransportError,
                "Resident Codex app-server stdio is unavailable.",
            ):
                transport.start()

        self.assertIsNone(transport.process)
        self.assertFalse(process.terminated)
        self.assertEqual(process.wait_timeout, 1.5)

    def test_close_locked_resets_owner_state_and_delegates_process_close(self) -> None:
        process = _process()
        calls: list[subprocess.Popen[str]] = []
        transport = _StartProbeTransport(executable="codex.exe")
        transport.install_process(process, initialized=True)
        transport.mark_thread_subscribed("thread-1")

        def close_process(process_to_close: subprocess.Popen[str], _log: Callable[[str], None]) -> None:
            calls.append(process_to_close)

        with mock.patch.object(resident_mod, "close_resident_app_server_process", close_process):
            transport.close_locked()

        self.assertEqual(calls, [process])
        self.assertIsNone(transport.process)
        self.assertFalse(transport.initialized)
        self.assertFalse(transport.is_thread_subscribed("thread-1"))

    def test_close_failure_preserves_process_subscription_and_child_cleanup_debt(self) -> None:
        process = _process()
        transport = _StartProbeTransport(executable="codex.exe")
        transport.install_process(process, initialized=True)
        transport._generation = 17
        transport._children.reset(17)
        transport.mark_thread_subscribed("thread-1")
        transport._handle_raw_line(
            json.dumps(
                {
                    "method": "thread/started",
                    "params": {
                        "thread": {
                            "id": "child-1",
                            "parentThreadId": "thread-1",
                        }
                    },
                }
            )
        )

        with mock.patch.object(
            resident_mod,
            "close_resident_app_server_process",
            side_effect=OSError("job close failed"),
        ):
            with self.assertRaisesRegex(OSError, "job close failed"):
                transport.close_locked()

        self.assertIs(transport.process, process)
        self.assertTrue(transport.is_thread_subscribed("thread-1"))
        self.assertTrue(transport.child_lifecycle_snapshot().cleanup_pending)

    def test_first_generation_uses_the_process_unique_seed(self) -> None:
        process = _process()
        transport = _StartProbeTransport(
            executable="codex.exe",
            generation_seed_func=lambda: 9000,
        )

        with (
            mock.patch.object(resident_mod, "start_resident_app_server_process", return_value=process),
            mock.patch.object(threading, "Thread", _FakeThread),
        ):
            transport.start()

        self.assertEqual(transport.lifecycle_snapshot().generation, 9000)


class ResidentTransportTimeoutBoundaryTests(unittest.TestCase):
    def test_request_slot_wait_uses_the_request_timeout_budget(self) -> None:
        transport = _RequestBoundaryProbe()
        transport.hold_request_slot()
        try:
            with self.assertRaisesRegex(TimeoutError, "request slot for thread/read"):
                _ = transport.request_started("thread/read", timeout_sec=0.01)
        finally:
            transport.release_request_slot()

        self.assertEqual(transport.written_messages, [])

    def test_response_timeout_quarantines_generation_and_rejects_new_delivery(self) -> None:
        transport = _RequestBoundaryProbe()
        process = _process()
        restarted = threading.Event()
        transport.install_lifecycle(process, generation=1)
        transport.seed_active_turn()

        with mock.patch.object(
            transport,
            "_start_with_request_slot_acquired",
            side_effect=restarted.set,
        ):
            with self.assertRaisesRegex(TimeoutError, "response to turn/start"):
                _ = transport.request(
                    "turn/start",
                    {"threadId": "thread-1"},
                    timeout_sec=0.01,
                    expected_generation=1,
                )

            snapshot = transport.lifecycle_snapshot()
            self.assertFalse(snapshot.healthy)
            self.assertTrue(snapshot.quarantined)
            self.assertTrue(snapshot.restart_pending)
            self.assertIs(transport.process, process)
            self.assertIsNone(process.poll())
            self.assertFalse(restarted.wait(timeout=0.05))
            with self.assertRaises(transport_mod.AppServerGenerationExpiredError):
                with transport.delivery_admission(1):
                    self.fail("quarantined generation admitted a new delivery")
            self.assertEqual(
                [message["method"] for message in transport.written_messages],
                ["turn/start"],
            )

            transport.clear_active_turn()
            transport.notify_child_cleanup_blocker_changed()
            self.assertFalse(restarted.wait(timeout=0.05))
            transport.resolve_ambiguous_turn_start("thread-1")
            self.assertTrue(restarted.wait(timeout=2.0))

    def test_thread_read_timeout_keeps_generation_healthy_for_other_deliveries(
        self,
    ) -> None:
        transport = _RequestBoundaryProbe()
        transport.install_lifecycle(_process(), generation=1)
        transport.seed_active_turn()
        transport._quarantine_after_response_timeout(
            "thread/read",
            {"threadId": "thread-1"},
            "request-1",
        )

        with transport.delivery_admission(1) as snapshot:
            self.assertTrue(snapshot.healthy)
            self.assertFalse(snapshot.quarantined)
            self.assertFalse(snapshot.restart_pending)
            self.assertEqual(snapshot.consecutive_read_timeouts, 1)
            self.assertFalse(snapshot.read_degraded)

        transport.stop_restart_retry()

    def test_thread_read_timeout_does_not_restart_or_retry_the_generation(self) -> None:
        transport = _ReadRecoveryProbe()
        transport.install_lifecycle(_process(), generation=1)

        with self.assertRaisesRegex(TimeoutError, "thread/read timed out"):
            _ = transport.request(
                "thread/read",
                {"threadId": "thread-1"},
                timeout_sec=8.0,
            )

        self.assertEqual(transport.restart_calls, 0)
        self.assertEqual(transport.requests, [("thread/read", 8.0, None)])
        self.assertTrue(transport.lifecycle_snapshot().healthy)

    def test_successful_thread_read_clears_degraded_read_state(self) -> None:
        transport = _ReadRecoveryProbe()
        transport.install_lifecycle(_process(), generation=1)

        with self.assertRaisesRegex(TimeoutError, "thread/read timed out"):
            _ = transport.request(
                "thread/read",
                {"threadId": "thread-1"},
                timeout_sec=8.0,
            )
        transport._quarantine_after_response_timeout(
            "thread/read",
            {"threadId": "thread-2"},
            "request-2",
        )
        self.assertTrue(transport.lifecycle_snapshot().read_degraded)

        result = transport.request(
            "thread/read",
            {"threadId": "thread-1"},
            timeout_sec=8.0,
        )
        self.assertEqual(result, {"thread": {"id": "thread-1"}})
        snapshot = transport.lifecycle_snapshot()
        self.assertEqual(snapshot.consecutive_read_timeouts, 0)
        self.assertFalse(snapshot.read_degraded)

    def test_repeated_thread_read_timeouts_mark_only_read_channel_degraded(self) -> None:
        transport = _RequestBoundaryProbe()
        transport.install_lifecycle(_process(), generation=1)

        transport._quarantine_after_response_timeout(
            "thread/read",
            {"threadId": "thread-1"},
            "request-1",
        )
        transport._quarantine_after_response_timeout(
            "thread/read",
            {"threadId": "thread-2"},
            "request-2",
        )

        snapshot = transport.lifecycle_snapshot()
        self.assertTrue(snapshot.healthy)
        self.assertTrue(snapshot.read_degraded)
        self.assertEqual(snapshot.consecutive_read_timeouts, 2)

    def test_thread_read_does_not_claim_an_unrelated_delivery_lease(self) -> None:
        transport = _ReadRecoveryProbe()
        transport.install_lifecycle(_process(), generation=1)
        lease_entered = threading.Event()
        release_lease = threading.Event()

        def hold_unrelated_lease() -> None:
            with transport.delivery_admission(1):
                lease_entered.set()
                self.assertTrue(release_lease.wait(timeout=2.0))

        lease_thread = threading.Thread(target=hold_unrelated_lease)
        lease_thread.start()
        self.assertTrue(lease_entered.wait(timeout=2.0))
        try:
            with self.assertRaisesRegex(TimeoutError, "thread/read timed out"):
                _ = transport.request(
                    "thread/read",
                    {"threadId": "thread-1"},
                    timeout_sec=8.0,
                )
            self.assertEqual(transport.restart_calls, 0)
            self.assertEqual(transport.requests, [("thread/read", 8.0, None)])
        finally:
            release_lease.set()
            lease_thread.join(timeout=2.0)
            transport.stop_restart_retry()

    def test_thread_read_with_expected_generation_never_crosses_generation(self) -> None:
        transport = _ReadRecoveryProbe()
        transport.install_lifecycle(_process(), generation=1)
        transport.seed_active_turn()

        with self.assertRaisesRegex(TimeoutError, "thread/read timed out"):
            _ = transport.request(
                "thread/read",
                {"threadId": "thread-1"},
                timeout_sec=8.0,
                expected_generation=1,
            )

        self.assertEqual(transport.requests, [("thread/read", 8.0, 1)])
        transport.stop_restart_retry()

    def test_turn_start_timeout_never_replays_non_idempotent_request(self) -> None:
        transport = _ReadRecoveryProbe()
        transport.install_lifecycle(_process(), generation=1)
        transport.seed_active_turn()

        with self.assertRaisesRegex(TimeoutError, "turn/start timed out"):
            _ = transport.request(
                "turn/start",
                {"threadId": "thread-1"},
                timeout_sec=12.0,
                expected_generation=1,
            )

        self.assertEqual(transport.restart_calls, 0)
        self.assertEqual(transport.requests, [("turn/start", 12.0, 1)])
        transport.stop_restart_retry()

    def test_ambiguous_turn_start_restarts_after_signal_grace_expires(self) -> None:
        transport = _RequestBoundaryProbe()
        restarted = threading.Event()
        transport.install_lifecycle(_process(), generation=1)

        with mock.patch.object(
            transport,
            "_start_with_request_slot_acquired",
            side_effect=restarted.set,
        ):
            with self.assertRaisesRegex(TimeoutError, "response to turn/start"):
                _ = transport.request(
                    "turn/start",
                    {"threadId": "thread-1"},
                    timeout_sec=0.01,
                    expected_generation=1,
                )
            self.assertFalse(restarted.wait(timeout=0.05))

            transport.expire_ambiguous_turn_start()
            self.assertTrue(restarted.wait(timeout=2.0))

    def test_late_turn_start_error_response_releases_pending_restart(self) -> None:
        transport = _RequestBoundaryProbe()
        restarted = threading.Event()
        transport.install_lifecycle(_process(), generation=1)

        with mock.patch.object(
            transport,
            "_start_with_request_slot_acquired",
            side_effect=restarted.set,
        ):
            with self.assertRaisesRegex(TimeoutError, "response to turn/start"):
                _ = transport.request(
                    "turn/start",
                    {"threadId": "thread-1"},
                    timeout_sec=0.01,
                    expected_generation=1,
                )
            request_id = str(transport.written_messages[0]["id"])
            transport.handle_raw_line(
                json.dumps(
                    {
                        "id": request_id,
                        "error": {"code": -32000, "message": "not accepted"},
                    }
                )
            )

            self.assertTrue(restarted.wait(timeout=2.0))
            self.assertEqual(
                [message["method"] for message in transport.written_messages],
                ["turn/start"],
            )

    def test_late_idle_thread_read_reconciles_stale_active_turn_without_restart(self) -> None:
        transport = _RequestBoundaryProbe()
        restarted = threading.Event()
        transport.install_lifecycle(_process(), generation=1)
        transport.seed_active_turn()

        with mock.patch.object(
            transport,
            "_start_with_request_slot_acquired",
            side_effect=restarted.set,
        ):
            with self.assertRaisesRegex(TimeoutError, "response to thread/read"):
                _ = transport.request(
                    "thread/read",
                    {"threadId": "thread-1", "includeTurns": False},
                    timeout_sec=0.01,
                    expected_generation=1,
                )
            request_id = str(transport.written_messages[0]["id"])

            transport.handle_raw_line(
                json.dumps(
                    {
                        "id": request_id,
                        "result": {
                            "thread": {
                                "id": "thread-1",
                                "status": {"type": "idle"},
                            }
                        },
                    }
                )
            )

            self.assertFalse(restarted.wait(timeout=0.05))
            self.assertIsNone(transport._pending.active_turn_id("thread-1"))

    def test_late_in_progress_thread_read_keeps_active_turn_and_defers_restart(self) -> None:
        transport = _RequestBoundaryProbe()
        restarted = threading.Event()
        transport.install_lifecycle(_process(), generation=1)
        transport.seed_active_turn()

        with mock.patch.object(
            transport,
            "_start_with_request_slot_acquired",
            side_effect=restarted.set,
        ):
            with self.assertRaisesRegex(TimeoutError, "response to thread/read"):
                _ = transport.request(
                    "thread/read",
                    {"threadId": "thread-1", "includeTurns": False},
                    timeout_sec=0.01,
                    expected_generation=1,
                )
            request_id = str(transport.written_messages[0]["id"])

            transport.handle_raw_line(
                json.dumps(
                    {
                        "id": request_id,
                        "result": {
                            "thread": {
                                "id": "thread-1",
                                "status": {"type": "active"},
                            }
                        },
                    }
                )
            )

            self.assertFalse(restarted.wait(timeout=0.05))

        transport.stop_restart_retry()

    def test_pending_restart_waits_for_delivery_exit_then_retries_automatically(self) -> None:
        transport = _RequestBoundaryProbe()
        restarted = threading.Event()
        transport.install_lifecycle(_process(), generation=1)

        with mock.patch.object(
            transport,
            "_start_with_request_slot_acquired",
            side_effect=restarted.set,
        ):
            with transport.delivery_admission(1):
                with self.assertRaisesRegex(TimeoutError, "response to thread/goal/get"):
                    _ = transport.request(
                        "thread/goal/get",
                        {"threadId": "thread-1"},
                        timeout_sec=0.01,
                        expected_generation=1,
                    )
                self.assertFalse(restarted.wait(timeout=0.05))

            self.assertTrue(restarted.wait(timeout=2.0))

    def test_pending_restart_waits_for_server_request_and_external_work(self) -> None:
        for blocker in ("server_request", "external_work"):
            with self.subTest(blocker=blocker):
                transport = _RequestBoundaryProbe()
                restarted = threading.Event()
                external_work = [blocker == "external_work"]
                transport.install_lifecycle(_process(), generation=1)
                if blocker == "server_request":
                    transport.seed_pending_server_request()
                else:
                    transport.set_external_work_guard(lambda: external_work[0])

                with mock.patch.object(
                    transport,
                    "_start_with_request_slot_acquired",
                    side_effect=restarted.set,
                ):
                    with self.assertRaisesRegex(TimeoutError, "response to thread/goal/get"):
                        _ = transport.request(
                            "thread/goal/get",
                            {"threadId": "thread-1"},
                            timeout_sec=0.01,
                            expected_generation=1,
                        )
                    self.assertFalse(restarted.wait(timeout=0.05))

                    if blocker == "server_request":
                        transport.resolve_pending_server_request()
                    else:
                        external_work[0] = False
                        transport.notify_child_cleanup_blocker_changed()
                    self.assertTrue(restarted.wait(timeout=2.0))

    def test_pending_restart_ignores_subscriptions_when_other_work_is_quiescent(self) -> None:
        transport = _RequestBoundaryProbe()
        restarted = threading.Event()
        transport.install_lifecycle(_process(), generation=1)
        transport.mark_thread_subscribed("thread-1")

        with mock.patch.object(
            transport,
            "_start_with_request_slot_acquired",
            side_effect=restarted.set,
        ):
            with self.assertRaisesRegex(TimeoutError, "response to thread/goal/get"):
                _ = transport.request(
                    "thread/goal/get",
                    {"threadId": "thread-1"},
                    timeout_sec=0.01,
                    expected_generation=1,
                )

            self.assertTrue(restarted.wait(timeout=2.0))
            self.assertFalse(transport.is_thread_subscribed("thread-1"))

    def test_explicit_close_prevents_pending_retry_from_restarting_server(self) -> None:
        transport = _RequestBoundaryProbe()
        transport.install_lifecycle(_process(), generation=1)
        with transport._condition:
            transport._quarantined_generation = 1
            transport._restart_pending = True

        transport.close()
        settled = transport._retry_restart_pending_once(1)

        self.assertTrue(settled)
        self.assertIsNone(transport.process)
        self.assertFalse(transport.is_running())

    def test_expected_generation_does_not_auto_start_an_unhealthy_server(self) -> None:
        resolver_calls: list[bool] = []
        transport = ResidentCodexAppServerTransport(
            executable_resolver=lambda: resolver_calls.append(True) or "codex.exe"
        )

        with self.assertRaisesRegex(transport_mod.AppServerGenerationExpiredError, "expected generation 1"):
            _ = transport.request("thread/read", {}, expected_generation=1)

        self.assertEqual(resolver_calls, [])

    def test_expected_generation_is_checked_before_request_write(self) -> None:
        transport = _RequestBoundaryProbe()
        transport.install_lifecycle(_process(), generation=2)

        with self.assertRaisesRegex(transport_mod.AppServerGenerationExpiredError, "found generation 2"):
            _ = transport.request("turn/start", {}, expected_generation=1)

        self.assertEqual(transport.written_messages, [])

    def test_expected_generation_is_rechecked_after_waiting_for_request_slot(self) -> None:
        transport = _RequestBoundaryProbe()
        transport.install_lifecycle(_process(), generation=1)
        errors: list[Exception] = []
        transport.hold_request_slot()

        def request_after_slot() -> None:
            try:
                _ = transport.request("turn/start", {}, expected_generation=1)
            except Exception as exc:  # noqa: BLE001 - assertion captures the lifecycle error.
                errors.append(exc)

        request_thread = threading.Thread(target=request_after_slot)
        request_thread.start()
        transport.set_generation(2)
        transport.release_request_slot()
        request_thread.join(timeout=2.0)

        self.assertEqual(len(errors), 1)
        self.assertIsInstance(errors[0], transport_mod.AppServerGenerationExpiredError)
        self.assertEqual(transport.written_messages, [])

    def test_turn_completion_wait_rejects_expired_generation(self) -> None:
        transport = _RequestBoundaryProbe()
        transport.install_lifecycle(_process(), generation=2)

        with self.assertRaisesRegex(transport_mod.AppServerGenerationExpiredError, "found generation 2"):
            _ = transport.wait_for_turn_completion(
                "thread-1",
                "turn-1",
                timeout_sec=0.0,
                expected_generation=1,
            )

    def test_turn_completion_wait_accepts_current_healthy_generation(self) -> None:
        transport = _RequestBoundaryProbe()
        transport.install_lifecycle(_process(), generation=2)

        observation = transport.wait_for_turn_completion(
            "thread-1",
            "turn-1",
            timeout_sec=0.0,
            expected_generation=2,
        )

        self.assertIsInstance(observation, TurnCompletionPending)

    def test_response_for_active_request_is_recorded(self) -> None:
        transport = _RequestBoundaryProbe()
        transport.seed_active_request("active-request")

        transport.handle_raw_line('{"id":"active-request","result":{"thread":{"id":"thread-1"}}}')

        self.assertEqual(
            transport.responses_snapshot(),
            {"active-request": {"id": "active-request", "result": {"thread": {"id": "thread-1"}}}},
        )

    def test_unmatched_response_is_discarded_without_a_timeout_tombstone(self) -> None:
        logs: list[str] = []
        transport = _RequestBoundaryProbe(log_func=logs.append)

        transport.handle_raw_line('{"id":"late-request","result":{"thread":{"id":"thread-1"}}}')

        self.assertEqual(transport.responses_snapshot(), {})
        self.assertEqual(logs, ["app_server_transport_late_response_discarded id=late-request"])

    def test_old_process_reader_cannot_mutate_the_current_generation(self) -> None:
        logs: list[str] = []
        transport = _RequestBoundaryProbe(log_func=logs.append)
        old_process = _process()
        current_process = _process()
        transport.install_lifecycle(current_process, generation=22)

        transport._handle_raw_line(
            json.dumps(
                {
                    "method": "thread/started",
                    "params": {
                        "thread": {
                            "id": "old-child",
                            "parentThreadId": "old-root",
                        }
                    },
                }
            ),
            source_process=old_process,
        )

        self.assertEqual(transport.child_lifecycle_snapshot().child_thread_ids, ())
        self.assertTrue(
            any("app_server_transport_stale_reader_line_discarded" in line for line in logs)
        )


class ResidentThreadResumeRetryTests(unittest.TestCase):
    def test_resume_retries_once_with_remaining_budget_after_first_timeout(self) -> None:
        clock_values = iter((100.0, 110.0))
        logs: list[str] = []
        requests: list[tuple[str, JsonMapping, float]] = []
        transport = transport_mod.PersistentCodexAppServer(
            executable_resolver=lambda: "codex.exe",
            log_func=logs.append,
            monotonic_func=lambda: next(clock_values),
        )

        def request(
            method: str,
            params: JsonMapping | None = None,
            *,
            timeout_sec: float = 10.0,
        ) -> JsonObject:
            requests.append((method, dict(params or {}), timeout_sec))
            if len(requests) == 1:
                raise TimeoutError("first resume timed out")
            return {"thread": {"id": "thread-1"}}

        with mock.patch.object(transport, "request", request):
            result = transport.resume_thread("thread-1", timeout_sec=32.0)

        self.assertEqual(result["thread"], {"id": "thread-1"})
        self.assertEqual(
            requests,
            [
                ("thread/resume", {"threadId": "thread-1"}, 10.0),
                ("thread/resume", {"threadId": "thread-1"}, 22.0),
            ],
        )
        self.assertEqual(len(logs), 1)
        self.assertIn("app_server_thread_resume_retry", logs[0])
        self.assertIn("thread=thread-1", logs[0])
        self.assertTrue(transport.is_thread_subscribed("thread-1"))

    def test_transport_methods_forward_expected_generation_to_requests(self) -> None:
        transport = transport_mod.PersistentCodexAppServer(executable_resolver=lambda: "codex.exe")

        with mock.patch.object(
            transport,
            "request",
            return_value={"thread": {"id": "thread-1", "turns": []}, "turn": {"id": "turn-1"}},
        ) as request:
            _ = transport.read_thread("thread-1", expected_generation=7)
            _ = transport.resume_thread("thread-1", expected_generation=7)
            _ = transport.start_turn("thread-1", "hello", expected_generation=7)
            _ = transport.steer_turn(
                "thread-1",
                "hello",
                expected_turn_id="turn-1",
                expected_generation=7,
            )
            _ = transport.get_thread_turn_states("thread-1", expected_generation=7)

        self.assertEqual([call.kwargs["expected_generation"] for call in request.call_args_list], [7, 7, 7, 7, 7])


class _StartProbeTransport(ResidentCodexAppServerTransport):
    def __init__(
        self,
        *,
        executable: str,
        log_func: Callable[[str], None] | None = None,
        wall_time_func: Callable[[], float] = time.time,
        generation_seed_func: Callable[[], int] = lambda: 1,
        plugin_runtime_fingerprint_reader: Callable[[], str] = (
            lambda: "test-fingerprint"
        ),
    ) -> None:
        super().__init__(
            executable_resolver=lambda: executable,
            log_func=log_func,
            wall_time_func=wall_time_func,
            generation_seed_func=generation_seed_func,
            plugin_runtime_fingerprint_reader=plugin_runtime_fingerprint_reader,
        )
        self.requests: list[tuple[str, JsonMapping, float]] = []
        self.notifications: list[tuple[str, JsonMapping]] = []

    def seed_stale_state(self, log: Callable[[str], None]) -> None:
        self._responses["old"] = {"stale": True}
        self._pending.record_server_request(
            "old",
            {"method": "item/tool/requestUserInput", "params": {"threadId": "thread-1"}},
            log,
        )

    def responses_snapshot(self) -> dict[str, JsonObject]:
        return dict(self._responses)

    def drain_process(self, process: process_mod.ResidentProcess) -> None:
        self._drain_stdout(process)

    def install_process(
        self,
        process: process_mod.ResidentProcess,
        *,
        initialized: bool,
    ) -> None:
        self.process = process
        self._initialized: bool = initialized

    @property
    def initialized(self) -> bool:
        return self._initialized

    @property
    def stdout_thread_started(self) -> bool:
        return isinstance(self._stdout_thread, _FakeThread) and self._stdout_thread.started

    @override
    def _request_started(
        self,
        method: str,
        params: JsonMapping,
        *,
        timeout_sec: float,
        expected_generation: int | None = None,
        _request_slot_acquired: bool = False,
    ) -> JsonObject:
        _ = (expected_generation, _request_slot_acquired)
        self.requests.append((method, dict(params), timeout_sec))
        return {}

    @override
    def notify(self, method: str, params: JsonMapping | None = None) -> None:
        self.notifications.append((method, dict(params or {})))


class _RequestBoundaryProbe(ResidentCodexAppServerTransport):
    process: process_mod.ResidentProcess | None
    _initialized: bool
    _generation: int
    _accepting_since: float | None
    _active_request_id: str | None
    _ambiguous_turn_start_deadline: float | None

    def __init__(self, *, log_func: Callable[[str], None] | None = None) -> None:
        super().__init__(executable_resolver=lambda: "codex.exe", log_func=log_func)
        self.written_messages: list[JsonObject] = []

    def hold_request_slot(self) -> None:
        _ = self._request_lock.acquire()

    def release_request_slot(self) -> None:
        self._request_lock.release()

    def request_started(self, method: str, *, timeout_sec: float) -> JsonObject:
        return self._request_started(method, {}, timeout_sec=timeout_sec)

    def install_lifecycle(self, process: subprocess.Popen[str], *, generation: int) -> None:
        self.process = process
        self._initialized = True
        self._generation = generation
        self._accepting_since = 100.0

    def set_generation(self, generation: int) -> None:
        with self._lock:
            self._generation = generation

    def seed_active_request(self, request_id: str) -> None:
        self._active_request_id = request_id

    def seed_active_turn(self) -> None:
        self._pending.record_notification(
            {
                "method": "turn/started",
                "params": {"threadId": "thread-1", "turnId": "turn-1"},
            },
            lambda _line: None,
            now=self.monotonic_func(),
        )

    def clear_active_turn(self) -> None:
        self._pending.active_turns.clear()

    def resolve_ambiguous_turn_start(self, thread_id: str) -> None:
        with self._condition:
            self._resolve_ambiguous_turn_start_from_notification(
                {
                    "method": "turn/completed",
                    "params": {"threadId": thread_id},
                }
            )
            self._condition.notify_all()
        self._restart_retry.wake()

    def expire_ambiguous_turn_start(self) -> None:
        with self._condition:
            self._ambiguous_turn_start_deadline = self.monotonic_func() - 1.0
            self._condition.notify_all()
        self._restart_retry.wake()

    def seed_pending_server_request(self) -> None:
        self._pending.record_server_request(
            "approval-1",
            {
                "id": "approval-1",
                "method": "item/tool/requestUserInput",
                "params": {"threadId": "thread-1"},
            },
            lambda _line: None,
        )

    def resolve_pending_server_request(self) -> None:
        self._pending.resolve_request("approval-1")
        self.notify_child_cleanup_blocker_changed()

    def stop_restart_retry(self) -> None:
        self._restart_retry.stop()

    def handle_raw_line(self, raw_line: str) -> None:
        self._handle_raw_line(raw_line)

    def responses_snapshot(self) -> dict[str, JsonObject]:
        return dict(self._responses)

    @override
    def _write_message(self, payload: JsonMapping) -> None:
        self.written_messages.append(dict(payload))


@final
class _ReadRecoveryProbe(_RequestBoundaryProbe):
    def __init__(self) -> None:
        super().__init__()
        self.requests: list[tuple[str, float, int | None]] = []
        self.restart_calls = 0

    @override
    def _request_started(
        self,
        method: str,
        params: JsonMapping,
        *,
        timeout_sec: float,
        expected_generation: int | None = None,
        _request_slot_acquired: bool = False,
    ) -> JsonObject:
        _ = (params, _request_slot_acquired)
        self.requests.append((method, timeout_sec, expected_generation))
        if len(self.requests) == 1:
            self._quarantine_after_response_timeout(method, params, "request-1")
            raise TimeoutError(f"{method} timed out")
        if method == "thread/read":
            self._record_thread_read_response()
        return {"thread": {"id": "thread-1"}}

    @override
    def _start_with_request_slot_acquired(self) -> None:
        if self._is_running() and self._initialized and not self._restart_pending:
            return
        self.restart_calls += 1
        with self._condition:
            self.process = _process()
            self._generation += 1
            self._initialized = True
            self._draining = False
            self._quarantined_generation = None
            self._restart_pending = False
            self._condition.notify_all()


class _FakeThread:
    def __init__(self, *, target: Callable[[], None], daemon: bool) -> None:
        self.target: Callable[[], None] = target
        self.daemon: bool = daemon
        self.started: bool = False

    def start(self) -> None:
        self.started = True

    def join(self, timeout: float | None = None) -> None:
        _ = timeout

    def is_alive(self) -> bool:
        return False


_DEFAULT_PIPE: Final = object()


class _FakeProcess:
    def __init__(
        self,
        *,
        stdin: IO[str] | None | object = _DEFAULT_PIPE,
        stdout: IO[str] | None | object = _DEFAULT_PIPE,
        poll_result: int | None = None,
        terminate_error: Exception | None = None,
        wait_error: Exception | None = None,
        kill_error: Exception | None = None,
        pid: int = 4242,
    ) -> None:
        self.stdin: IO[str] | None = io.StringIO() if stdin is _DEFAULT_PIPE else cast(IO[str] | None, stdin)
        self.stdout: IO[str] | None = io.StringIO() if stdout is _DEFAULT_PIPE else cast(IO[str] | None, stdout)
        self.stderr: IO[str] | None = io.StringIO()
        self.poll_result: int | None = poll_result
        self.terminate_error: Exception | None = terminate_error
        self.wait_error: Exception | None = wait_error
        self.kill_error: Exception | None = kill_error
        self.pid: int = pid
        self.terminated: bool = False
        self.killed: bool = False
        self.wait_timeout: float | None = None

    def poll(self) -> int | None:
        if self.terminated:
            return 0
        return self.poll_result

    def terminate(self) -> None:
        if self.terminate_error is not None:
            raise self.terminate_error
        self.terminated = True

    def wait(self, timeout: float | None = None) -> int:
        self.wait_timeout = timeout
        if self.wait_error is not None:
            raise self.wait_error
        return 0

    def kill(self) -> None:
        self.killed = True
        if self.kill_error is not None:
            raise self.kill_error


class _FailingCloseStdin(io.StringIO):
    @override
    def close(self) -> None:
        raise OSError("close boom")


def _process(
    *,
    stdin: IO[str] | None | object = _DEFAULT_PIPE,
    stdout: IO[str] | None | object = _DEFAULT_PIPE,
) -> subprocess.Popen[str]:
    return _as_popen(_FakeProcess(stdin=stdin, stdout=stdout))


def _as_popen(process: _FakeProcess) -> subprocess.Popen[str]:
    return cast(subprocess.Popen[str], cast(object, process))


if __name__ == "__main__":
    _ = unittest.main()
