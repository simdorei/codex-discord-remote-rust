from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
import unittest

from codex_app_server_transport_lifecycle import AppServerLifecycleSnapshot
from codex_app_server_transport_replies import CodexAppServerTransportError
from codex_app_server_transport_subscriptions import (
    ThreadReleaseOutcome,
    ThreadReleaseStatus,
)
import codex_discord_app_server_subscription_release as subscription_release


@dataclass(slots=True)
class FakeClient:
    snapshot: AppServerLifecycleSnapshot
    outcome: ThreadReleaseOutcome = ThreadReleaseOutcome(ThreadReleaseStatus.RELEASED)
    error: Exception | None = None
    calls: list[tuple[str, int]] | None = None
    on_release: Callable[[], None] | None = None

    def lifecycle_snapshot(self) -> AppServerLifecycleSnapshot:
        return self.snapshot

    def release_thread_subscription_if_terminal(
        self,
        thread_id: str,
        *,
        expected_generation: int,
    ) -> ThreadReleaseOutcome:
        if self.calls is not None:
            self.calls.append((thread_id, expected_generation))
        if self.error is not None:
            raise self.error
        if self.on_release is not None:
            self.on_release()
        return self.outcome


class AppServerSubscriptionReleaseTests(unittest.IsolatedAsyncioTestCase):
    async def test_success_releases_subscription_before_deactivation(self) -> None:
        events: list[str] = []
        calls: list[tuple[str, int]] = []
        activation: list[float | None] = [1.0]
        client = FakeClient(AppServerLifecycleSnapshot(7, True, 1.0), calls=calls)

        def deactivate_if_unchanged(_thread_id: str | None, expected: float | None) -> bool:
            if activation[0] != expected:
                return False
            activation[0] = None
            events.append("deactivate:thread-1")
            return True

        released = await subscription_release.release_session_mirror_output_target(
            "thread-1",
            transport_enabled=lambda: True,
            client=client,
            get_output_target_activation=lambda _thread_id: activation[0],
            deactivate_output_target_if_unchanged=deactivate_if_unchanged,
            log=events.append,
        )

        self.assertTrue(released)
        self.assertEqual(calls, [("thread-1", 7)])
        self.assertEqual(events, ["deactivate:thread-1"])

    async def test_unhealthy_transport_preserves_output_target(self) -> None:
        events: list[str] = []
        activation: list[float | None] = [1.0]
        client = FakeClient(AppServerLifecycleSnapshot(7, False, None))

        released = await subscription_release.release_session_mirror_output_target(
            "thread-1",
            transport_enabled=lambda: True,
            client=client,
            get_output_target_activation=lambda _thread_id: activation[0],
            deactivate_output_target_if_unchanged=lambda _thread_id, _expected: False,
            log=events.append,
        )

        self.assertFalse(released)
        self.assertFalse(any(event.startswith("deactivate:") for event in events))
        self.assertTrue(any("app_server_unhealthy" in event for event in events))

    async def test_transport_error_is_logged_and_preserves_output_target(self) -> None:
        events: list[str] = []
        activation: list[float | None] = [1.0]
        client = FakeClient(
            AppServerLifecycleSnapshot(7, True, 1.0),
            error=CodexAppServerTransportError("generation changed"),
        )

        released = await subscription_release.release_session_mirror_output_target(
            "thread-1",
            transport_enabled=lambda: True,
            client=client,
            get_output_target_activation=lambda _thread_id: activation[0],
            deactivate_output_target_if_unchanged=lambda _thread_id, _expected: False,
            log=events.append,
        )

        self.assertFalse(released)
        self.assertFalse(any(event.startswith("deactivate:") for event in events))
        self.assertTrue(any("generation changed" in event for event in events))

    async def test_reactivation_during_release_preserves_new_output_target(self) -> None:
        events: list[str] = []
        activation: list[float | None] = [1.0]
        client = FakeClient(
            AppServerLifecycleSnapshot(7, True, 1.0),
            on_release=lambda: activation.__setitem__(0, 2.0),
        )

        released = await subscription_release.release_session_mirror_output_target(
            "thread-1",
            transport_enabled=lambda: True,
            client=client,
            get_output_target_activation=lambda _thread_id: activation[0],
            deactivate_output_target_if_unchanged=(
                lambda _thread_id, expected: activation[0] == expected
            ),
            log=events.append,
        )

        self.assertFalse(released)
        self.assertEqual(activation[0], 2.0)
        self.assertTrue(any("reactivated_after_release" in event for event in events))

    async def test_expiry_token_changed_before_release_does_not_unsubscribe_new_target(self) -> None:
        events: list[str] = []
        calls: list[tuple[str, int]] = []
        client = FakeClient(AppServerLifecycleSnapshot(7, True, 1.0), calls=calls)

        released = await subscription_release.release_session_mirror_output_target(
            "thread-1",
            transport_enabled=lambda: True,
            client=client,
            get_output_target_activation=lambda _thread_id: 2.0,
            deactivate_output_target_if_unchanged=lambda _thread_id, _expected: False,
            log=events.append,
            expected_activation=1.0,
        )

        self.assertFalse(released)
        self.assertEqual(calls, [])
        self.assertTrue(any("reactivated_before_release" in event for event in events))


if __name__ == "__main__":
    _ = unittest.main()
