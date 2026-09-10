from __future__ import annotations

import threading
import unittest
from typing import final

from codex_app_server_transport_goal import (
    GoalAbsent,
    GoalPresent,
    GoalTransportError,
    ThreadGoalLookup,
    ThreadGoalStatus,
)
from codex_app_server_transport_replies import CodexAppServerTransportError, JsonMapping, JsonObject
from codex_app_server_transport_subscriptions import (
    ThreadReleaseStatus,
    ThreadSubscriptionCoordinator,
)


class ThreadSubscriptionCoordinatorTests(unittest.TestCase):
    def test_terminal_thread_unsubscribes_and_forgets_subscription(self) -> None:
        coordinator = ThreadSubscriptionCoordinator()
        client = _ReleaseClient()
        logs: list[str] = []
        coordinator.mark_subscribed("thread-1")

        outcome = coordinator.release_if_terminal(client, "thread-1", log=logs.append)

        self.assertEqual(outcome.status, ThreadReleaseStatus.RELEASED)
        self.assertFalse(coordinator.is_subscribed("thread-1"))
        self.assertEqual(
            client.requests,
            [("thread/unsubscribe", {"threadId": "thread-1"}, 8.0)],
        )
        self.assertEqual(client.expected_generations, [None, None, None])
        self.assertEqual(logs, ["app_server_thread_unsubscribed target=thread-1"])

    def test_expected_generation_is_forwarded_to_every_app_server_check(self) -> None:
        coordinator = ThreadSubscriptionCoordinator()
        client = _ReleaseClient()
        coordinator.mark_subscribed("thread-1")

        outcome = coordinator.release_if_terminal(
            client,
            "thread-1",
            expected_generation=7,
            log=lambda _line: None,
        )

        self.assertEqual(outcome.status, ThreadReleaseStatus.RELEASED)
        self.assertEqual(client.expected_generations, [7, 7, 7])

    def test_already_unsubscribed_is_a_successful_release(self) -> None:
        coordinator = ThreadSubscriptionCoordinator()
        client = _ReleaseClient()

        outcome = coordinator.release_if_terminal(client, "thread-1", log=lambda _line: None)

        self.assertEqual(outcome.status, ThreadReleaseStatus.ALREADY_RELEASED)
        self.assertTrue(outcome.released)
        self.assertEqual(client.requests, [])

    def test_active_turn_and_active_goal_each_defer_release(self) -> None:
        cases = (
            (
                _ReleaseClient(
                    active_turn_id="turn-1",
                    pending_requests=[{"id": "approval-1"}],
                ),
                ThreadReleaseStatus.ACTIVE_TURN,
            ),
            (
                _ReleaseClient(
                    pending_requests=[{"id": "approval-1"}],
                    goal_lookup=GoalPresent(ThreadGoalStatus.ACTIVE),
                ),
                ThreadReleaseStatus.ACTIVE_GOAL,
            ),
        )
        for client, expected in cases:
            with self.subTest(expected=expected):
                coordinator = ThreadSubscriptionCoordinator()
                coordinator.mark_subscribed("thread-1")

                outcome = coordinator.release_if_terminal(client, "thread-1", log=lambda _line: None)

                self.assertEqual(outcome.status, expected)
                self.assertTrue(coordinator.is_subscribed("thread-1"))
                self.assertEqual(client.requests, [])
                self.assertEqual(client.cancelled_thread_ids, [])

    def test_inactive_thread_pending_requests_are_cancelled_before_release(self) -> None:
        coordinator = ThreadSubscriptionCoordinator()
        client = _ReleaseClient(pending_requests=[{"id": "approval-1"}])
        coordinator.mark_subscribed("thread-1")

        outcome = coordinator.release_if_terminal(client, "thread-1", log=lambda _line: None)

        self.assertEqual(outcome.status, ThreadReleaseStatus.RELEASED)
        self.assertEqual(client.pending_requests, [])
        self.assertEqual(client.cancelled_thread_ids, ["thread-1"])
        self.assertFalse(coordinator.is_subscribed("thread-1"))

    def test_complete_goal_allows_release(self) -> None:
        coordinator = ThreadSubscriptionCoordinator()
        client = _ReleaseClient(goal_lookup=GoalPresent(ThreadGoalStatus.COMPLETE))
        coordinator.mark_subscribed("thread-1")

        outcome = coordinator.release_if_terminal(client, "thread-1", log=lambda _line: None)

        self.assertEqual(outcome.status, ThreadReleaseStatus.RELEASED)

    def test_blocked_goal_without_active_turn_allows_release(self) -> None:
        coordinator = ThreadSubscriptionCoordinator()
        client = _ReleaseClient(goal_lookup=GoalPresent(ThreadGoalStatus.BLOCKED))
        coordinator.mark_subscribed("thread-1")

        outcome = coordinator.release_if_terminal(client, "thread-1", log=lambda _line: None)

        self.assertEqual(outcome.status, ThreadReleaseStatus.RELEASED)
        self.assertFalse(coordinator.is_subscribed("thread-1"))

    def test_transport_failure_is_logged_and_retried_after_backoff(self) -> None:
        clock = _Clock()
        coordinator = ThreadSubscriptionCoordinator(monotonic_func=clock)
        client = _ReleaseClient(unsubscribe_error=CodexAppServerTransportError("connection lost"))
        logs: list[str] = []
        coordinator.mark_subscribed("thread-1")

        failed = coordinator.release_if_terminal(client, "thread-1", log=logs.append)
        deferred = coordinator.release_if_terminal(client, "thread-1", log=logs.append)
        clock.value = 1.0
        client.unsubscribe_error = None
        released = coordinator.release_if_terminal(client, "thread-1", log=logs.append)

        self.assertEqual(failed.status, ThreadReleaseStatus.FAILED)
        self.assertEqual(deferred.status, ThreadReleaseStatus.RETRY_DEFERRED)
        self.assertEqual(released.status, ThreadReleaseStatus.RELEASED)
        self.assertIn("error_type=CodexAppServerTransportError error=connection lost", logs[0])
        self.assertIn("retry_after_sec=1", logs[0])
        self.assertEqual(len(client.requests), 2)

    def test_goal_lookup_error_uses_the_same_retry_path(self) -> None:
        coordinator = ThreadSubscriptionCoordinator()
        client = _ReleaseClient(goal_lookup=GoalTransportError("goal request timed out"))
        logs: list[str] = []
        coordinator.mark_subscribed("thread-1")

        outcome = coordinator.release_if_terminal(client, "thread-1", log=logs.append)

        self.assertEqual(outcome.status, ThreadReleaseStatus.FAILED)
        self.assertIn("error=goal request timed out", logs[0])
        self.assertEqual(client.requests, [])

    def test_new_activity_cancels_a_pending_release_backoff(self) -> None:
        coordinator = ThreadSubscriptionCoordinator()
        client = _ReleaseClient(unsubscribe_error=TimeoutError("slow"))
        coordinator.mark_subscribed("thread-1")
        _ = coordinator.release_if_terminal(client, "thread-1", log=lambda _line: None)

        coordinator.note_thread_activity("thread-1")
        client.unsubscribe_error = None
        outcome = coordinator.release_if_terminal(client, "thread-1", log=lambda _line: None)

        self.assertEqual(outcome.status, ThreadReleaseStatus.RELEASED)

    def test_new_turn_wins_race_before_release_can_unsubscribe(self) -> None:
        coordinator = ThreadSubscriptionCoordinator()
        client = _ReleaseClient()
        start_entered = threading.Event()
        allow_start = threading.Event()
        outcomes: list[ThreadReleaseStatus] = []
        coordinator.mark_subscribed("thread-1")

        def start_new_turn() -> None:
            with coordinator.lifecycle_lock("thread-1"):
                coordinator.note_thread_activity("thread-1")
                start_entered.set()
                self.assertTrue(allow_start.wait(timeout=2.0))
                client.active_turn_id = "turn-2"

        def release_old_turn() -> None:
            outcome = coordinator.release_if_terminal(client, "thread-1", log=lambda _line: None)
            outcomes.append(outcome.status)

        starter = threading.Thread(target=start_new_turn)
        releaser = threading.Thread(target=release_old_turn)
        starter.start()
        self.assertTrue(start_entered.wait(timeout=2.0))
        releaser.start()
        allow_start.set()
        starter.join(timeout=2.0)
        releaser.join(timeout=2.0)

        self.assertFalse(starter.is_alive())
        self.assertFalse(releaser.is_alive())
        self.assertEqual(outcomes, [ThreadReleaseStatus.ACTIVE_TURN])
        self.assertEqual(client.requests, [])
        self.assertTrue(coordinator.is_subscribed("thread-1"))


@final
class _Clock:
    def __init__(self) -> None:
        self.value = 0.0

    def __call__(self) -> float:
        return self.value


@final
class _ReleaseClient:
    def __init__(
        self,
        *,
        active_turn_id: str | None = None,
        pending_requests: list[JsonObject] | None = None,
        goal_lookup: ThreadGoalLookup | None = None,
        unsubscribe_error: Exception | None = None,
    ) -> None:
        self.active_turn_id = active_turn_id
        self.pending_requests = pending_requests or []
        self.goal_lookup = goal_lookup or GoalAbsent()
        self.unsubscribe_error = unsubscribe_error
        self.requests: list[tuple[str, JsonObject, float]] = []
        self.cancelled_thread_ids: list[str] = []
        self.expected_generations: list[int | None] = []

    def cancel_pending_server_requests(self, thread_id: str) -> int:
        self.cancelled_thread_ids.append(thread_id)
        cancelled = len(self.pending_requests)
        self.pending_requests.clear()
        return cancelled

    def has_active_turn_or_raise(
        self,
        thread_id: str,
        *,
        expected_generation: int | None = None,
    ) -> bool:
        _ = thread_id
        self.expected_generations.append(expected_generation)
        return self.active_turn_id is not None

    def get_pending_server_requests(self, thread_id: str | None = None) -> list[JsonObject]:
        _ = thread_id
        return list(self.pending_requests)

    def get_thread_goal_lookup(
        self,
        thread_id: str,
        *,
        timeout_sec: float = 3.0,
        expected_generation: int | None = None,
    ) -> ThreadGoalLookup:
        _ = thread_id, timeout_sec
        self.expected_generations.append(expected_generation)
        return self.goal_lookup

    def request(
        self,
        method: str,
        params: JsonMapping | None = None,
        *,
        timeout_sec: float = 10.0,
        expected_generation: int | None = None,
    ) -> JsonObject:
        self.expected_generations.append(expected_generation)
        self.requests.append((method, dict(params or {}), timeout_sec))
        if self.unsubscribe_error is not None:
            raise self.unsubscribe_error
        return {}


if __name__ == "__main__":
    _ = unittest.main()
