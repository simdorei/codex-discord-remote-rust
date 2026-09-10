from __future__ import annotations

import unittest

from codex_app_server_transport_turn_outcomes import TurnCompletion, TurnStatus
from codex_discord_queue_processor import (
    QueueAttempt,
    QueueJobSummary,
    QueueProcessResult,
    QueueTurnCoordinatorDeps,
    process_queue_job,
)
from codex_discord_runner_queue import QueueJob


class ProcessorFixture:
    def __init__(self, outcomes: list[TurnCompletion]) -> None:
        self.outcomes: list[TurnCompletion] = outcomes
        self.attempts: list[tuple[str, bool]] = []
        self.completed_job_ids: list[str] = []
        self.retry_reasons: list[str] = []
        self.batch_failures: list[tuple[str, list[QueueJobSummary]]] = []
        self.logs: list[str] = []

    async def acquire_turn(
        self,
        job: QueueJob,
        prompt: str,
        target_thread_id: str | None,
        *,
        recovery: bool,
    ) -> QueueAttempt:
        _ = job, target_thread_id
        self.attempts.append((prompt, recovery))
        attempt_number = len(self.attempts)
        return QueueAttempt(attempt_number, "thread-1", f"turn-{attempt_number}", 3)

    async def wait_for_completion(
        self,
        thread_id: str,
        turn_id: str,
        app_server_generation: int,
    ) -> TurnCompletion:
        _ = thread_id, turn_id, app_server_generation
        return self.outcomes.pop(0)

    async def complete_job(self, job: QueueJob) -> None:
        self.completed_job_ids.append(str(job.get("job_id") or ""))

    async def flush_jobs(
        self,
        job: QueueJob,
        target_thread_id: str | None,
    ) -> list[QueueJobSummary]:
        _ = job, target_thread_id
        return [
            QueueJobSummary("job-1", "first request"),
            QueueJobSummary("job-2", "second request"),
        ]

    async def report_retry(self, job: QueueJob, reason: str) -> None:
        _ = job
        self.retry_reasons.append(reason)

    async def report_batch_failure(
        self,
        job: QueueJob,
        reason: str,
        deleted_jobs: list[QueueJobSummary],
    ) -> None:
        _ = job
        self.batch_failures.append((reason, deleted_jobs))

    def deps(self) -> QueueTurnCoordinatorDeps:
        return QueueTurnCoordinatorDeps(
            acquire_turn=self.acquire_turn,
            wait_for_turn_completion=self.wait_for_completion,
            complete_job=self.complete_job,
            flush_jobs=self.flush_jobs,
            report_retry=self.report_retry,
            report_batch_failure=self.report_batch_failure,
            log=self.logs.append,
        )


def completion(status: TurnStatus, turn_id: str, *, error: str = "") -> TurnCompletion:
    return TurnCompletion(
        thread_id="thread-1",
        turn_id=turn_id,
        status=status,
        error_message=error,
    )


class QueueProcessorTests(unittest.IsolatedAsyncioTestCase):
    async def test_interrupted_turn_restarts_same_job_once_before_completing(self) -> None:
        fixture = ProcessorFixture(
            [
                completion(TurnStatus.INTERRUPTED, "turn-1"),
                completion(TurnStatus.COMPLETED, "turn-2"),
            ]
        )
        job: QueueJob = {"job_id": "job-1", "prompt": "apply migration"}

        result = await process_queue_job(job, "thread-1", deps=fixture.deps())

        self.assertIs(result, QueueProcessResult.COMPLETED)
        self.assertEqual(fixture.completed_job_ids, ["job-1"])
        self.assertEqual(len(fixture.attempts), 2)
        self.assertEqual(fixture.attempts[0], ("apply migration", False))
        recovery_prompt, recovery = fixture.attempts[1]
        self.assertTrue(recovery)
        self.assertIn("Inspect the existing thread and workspace state first", recovery_prompt)
        self.assertIn("apply migration", recovery_prompt)
        self.assertEqual(len(fixture.retry_reasons), 1)
        self.assertEqual(fixture.batch_failures, [])

    async def test_second_failed_turn_flushes_current_batch_with_reason_and_summaries(self) -> None:
        fixture = ProcessorFixture(
            [
                completion(TurnStatus.FAILED, "turn-1", error="first failure"),
                completion(TurnStatus.FAILED, "turn-2", error="second failure"),
            ]
        )
        job: QueueJob = {"job_id": "job-1", "prompt": "first request"}

        result = await process_queue_job(job, "thread-1", deps=fixture.deps())

        self.assertIs(result, QueueProcessResult.FLUSHED)
        self.assertEqual(fixture.completed_job_ids, [])
        self.assertEqual(len(fixture.attempts), 2)
        self.assertEqual(len(fixture.batch_failures), 1)
        reason, deleted_jobs = fixture.batch_failures[0]
        self.assertIn("second failure", reason)
        self.assertEqual([item.prompt for item in deleted_jobs], ["first request", "second request"])


if __name__ == "__main__":
    _ = unittest.main()
