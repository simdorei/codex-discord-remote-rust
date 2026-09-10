from __future__ import annotations

from dataclasses import dataclass, field
import unittest

from codex_app_server_transport_lifecycle import AppServerLifecycleSnapshot
from codex_app_server_transport_subscriptions import (
    ThreadReleaseOutcome,
    ThreadReleaseStatus,
)
import codex_discord_session_mirror_release_runtime as release_runtime


@dataclass(slots=True)
class FakeClient:
    calls: list[tuple[str, int]] = field(default_factory=list)

    def lifecycle_snapshot(self) -> AppServerLifecycleSnapshot:
        return AppServerLifecycleSnapshot(7, True, 1.0)

    def release_thread_subscription_if_terminal(
        self,
        thread_id: str,
        *,
        expected_generation: int,
    ) -> ThreadReleaseOutcome:
        self.calls.append((thread_id, expected_generation))
        return ThreadReleaseOutcome(ThreadReleaseStatus.RELEASED)


class SessionMirrorReleaseRuntimeTests(unittest.IsolatedAsyncioTestCase):
    async def test_expired_target_uses_subscription_release_before_compare_and_deactivate(self) -> None:
        client = FakeClient()
        activation: list[float | None] = [1.0]
        cleared: list[tuple[str, float]] = []

        def deactivate_if_unchanged(
            _thread_id: str | None,
            expected_activation: float | None,
        ) -> bool:
            if activation[0] != expected_activation:
                return False
            activation[0] = None
            return True

        runtime = release_runtime.SessionMirrorReleaseRuntime(
            release_runtime.SessionMirrorReleaseRuntimeDeps(
                transport_enabled=lambda: True,
                client=client,
                get_output_target_activation=lambda _thread_id: activation[0],
                deactivate_output_target_if_unchanged=deactivate_if_unchanged,
                clear_expiring_output_target=lambda thread_id, token: cleared.append(
                    (thread_id, token)
                ),
                log=lambda _message: None,
            )
        )

        await runtime._release_expired_output_target("thread-1", 1.0)

        self.assertEqual(client.calls, [("thread-1", 7)])
        self.assertIsNone(activation[0])
        self.assertEqual(cleared, [("thread-1", 1.0)])


class SessionMirrorReleaseRuntimeSyncTests(unittest.TestCase):
    def test_no_event_loop_preserves_target_and_clears_retry_marker(self) -> None:
        cleared: list[tuple[str, float]] = []
        logs: list[str] = []
        runtime = release_runtime.SessionMirrorReleaseRuntime(
            release_runtime.SessionMirrorReleaseRuntimeDeps(
                transport_enabled=lambda: True,
                client=FakeClient(),
                get_output_target_activation=lambda _thread_id: 1.0,
                deactivate_output_target_if_unchanged=lambda _thread_id, _token: False,
                clear_expiring_output_target=lambda thread_id, token: cleared.append(
                    (thread_id, token)
                ),
                log=logs.append,
            )
        )

        runtime.schedule_expired_output_target("thread-1", 1.0)

        self.assertEqual(cleared, [("thread-1", 1.0)])
        self.assertTrue(any("no_running_event_loop" in message for message in logs))


if __name__ == "__main__":
    _ = unittest.main()
