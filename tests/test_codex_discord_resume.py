from __future__ import annotations

import unittest
from typing import cast

import codex_discord_resume as resume
from codex_app_server_transport_replies import CodexAppServerTransportError, JsonObject


class ResidentThreadRecoveryTests(unittest.TestCase):
    def test_format_rejects_an_unexpected_runtime_state(self) -> None:
        result = resume.ResumeRecoveryResult(
            "thread-1",
            cast(resume.ResumeRecoveryState, cast(object, "unexpected")),
        )

        with self.assertRaises(AssertionError):
            _ = resume.format_resume_recovery_message(result, "project:1")

    def test_request_uses_mapped_channel_target(self) -> None:
        client = _RecoveryClient(("idle",))
        target_calls: list[tuple[int | None, str | None]] = []

        def resolve_queue_target(channel_id: int | None, ref: str | None) -> tuple[str | None, str]:
            target_calls.append((channel_id, ref))
            return "thread-1", "project:1"

        output = resume.recover_resident_thread_for_request(
            client,
            123,
            None,
            resolve_queue_command_target=resolve_queue_target,
            resolve_selected_target=lambda: ("unused", "unused"),
            monotonic_func=lambda: 100.0,
        )

        self.assertEqual(target_calls, [(123, None)])
        self.assertIn("thread: project:1", output)
        self.assertEqual(client.read_count, 1)

    def test_request_falls_back_to_selected_target(self) -> None:
        client = _RecoveryClient(("idle",))

        output = resume.recover_resident_thread_for_request(
            client,
            123,
            "selected",
            resolve_queue_command_target=lambda _channel_id, _ref: (None, "selected"),
            resolve_selected_target=lambda: ("thread-2", "chosen:2"),
            monotonic_func=lambda: 100.0,
        )

        self.assertIn("thread: chosen:2", output)

    def test_request_without_any_target_surfaces_actual_failure(self) -> None:
        client = _RecoveryClient(("idle",))

        with self.assertRaisesRegex(CodexAppServerTransportError, "no mapped or selected target"):
            _ = resume.recover_resident_thread_for_request(
                client,
                123,
                None,
                resolve_queue_command_target=lambda _channel_id, _ref: (None, "selected"),
                resolve_selected_target=lambda: (None, "selected"),
                monotonic_func=lambda: 100.0,
            )

    def test_already_loaded_thread_is_confirmed_without_resume(self) -> None:
        client = _RecoveryClient(("idle",))

        result = resume.recover_resident_thread(
            client,
            "thread-1",
            timeout_sec=30.0,
            monotonic_func=lambda: 100.0,
        )

        self.assertEqual(result.state, resume.ResumeRecoveryState.ALREADY_LOADED)
        self.assertEqual(client.resume_calls, [])
        self.assertEqual(client.read_count, 1)

    def test_not_loaded_thread_uses_default_sixty_second_budget_and_is_confirmed(self) -> None:
        client = _RecoveryClient(("notLoaded", "idle"))

        result = resume.recover_resident_thread(
            client,
            "thread-1",
            monotonic_func=lambda: 100.0,
        )

        self.assertEqual(result.state, resume.ResumeRecoveryState.RECOVERED)
        self.assertEqual(client.resume_calls, [("thread-1", 60.0)])
        self.assertEqual(client.read_count, 2)

    def test_read_resume_and_confirmation_share_one_sixty_second_budget(self) -> None:
        client = _RecoveryClient(("notLoaded", "idle"))
        clock_values = iter((100.0, 100.0, 108.0, 153.0))

        result = resume.recover_resident_thread(
            client,
            "thread-1",
            monotonic_func=lambda: next(clock_values),
        )

        self.assertEqual(result.state, resume.ResumeRecoveryState.RECOVERED)
        self.assertEqual(client.resume_calls, [("thread-1", 52.0)])
        self.assertEqual(client.read_timeouts, [8.0, 7.0])
        self.assertEqual(client.read_recovery_timeouts, [25.0, 0.0])

    def test_recovery_budget_reserves_restart_and_initialize_overhead(self) -> None:
        client = _RecoveryClient(("idle",))

        _ = resume.recover_resident_thread(
            client,
            "thread-1",
            timeout_sec=40.0,
            monotonic_func=lambda: 100.0,
        )

        self.assertEqual(client.read_timeouts, [8.0])
        self.assertEqual(client.read_recovery_timeouts, [17.0])

    def test_still_not_loaded_thread_surfaces_confirmation_failure(self) -> None:
        client = _RecoveryClient(("notLoaded", "notLoaded"))

        with self.assertRaisesRegex(CodexAppServerTransportError, "still not loaded"):
            _ = resume.recover_resident_thread(
                client,
                "thread-1",
                timeout_sec=30.0,
                monotonic_func=lambda: 100.0,
            )


class _RecoveryClient:
    def __init__(self, statuses: tuple[str, ...]) -> None:
        self.statuses: list[str] = list(statuses)
        self.read_count: int = 0
        self.read_timeouts: list[float] = []
        self.read_recovery_timeouts: list[float | None] = []
        self.resume_calls: list[tuple[str, float]] = []

    def read_thread(
        self,
        thread_id: str,
        *,
        include_turns: bool = False,
        timeout_sec: float = 8.0,
        recovery_timeout_sec: float | None = None,
    ) -> JsonObject:
        _ = include_turns
        self.read_count += 1
        self.read_timeouts.append(timeout_sec)
        self.read_recovery_timeouts.append(recovery_timeout_sec)
        status = self.statuses.pop(0)
        return {"thread": {"id": thread_id, "status": {"type": status}}}

    def resume_thread(self, thread_id: str, *, timeout_sec: float = 10.0) -> JsonObject:
        self.resume_calls.append((thread_id, timeout_sec))
        return {"thread": {"id": thread_id}}


if __name__ == "__main__":
    _ = unittest.main()
