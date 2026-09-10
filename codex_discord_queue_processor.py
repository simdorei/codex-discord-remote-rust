from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from enum import StrEnum, unique

from codex_app_server_transport_turn_outcomes import TurnCompletion, TurnStatus
from codex_discord_runner_queue import QueueJob


@dataclass(frozen=True, slots=True)
class QueueAttempt:
    attempt_number: int
    thread_id: str
    turn_id: str
    app_server_generation: int


@dataclass(frozen=True, slots=True)
class QueueJobSummary:
    job_id: str
    prompt: str


@unique
class QueueProcessResult(StrEnum):
    COMPLETED = "completed"
    FLUSHED = "flushed"


class QueueTurnOwnershipAmbiguousError(RuntimeError):
    pass


class QueueGenerationExpiredError(RuntimeError):
    def __init__(
        self,
        *,
        stage: str,
        expected_generation: int,
        current_generation: int,
        healthy: bool,
    ) -> None:
        current_label = str(current_generation) if healthy else "unhealthy"
        super().__init__(
            "Durable queue app-server generation expired "
            f"during {stage}: expected={expected_generation} current={current_label}"
        )
        self.stage = stage
        self.expected_generation = expected_generation
        self.current_generation = current_generation
        self.healthy = healthy


@dataclass(frozen=True, slots=True)
class QueueTurnCoordinatorDeps:
    acquire_turn: Callable[..., Awaitable[QueueAttempt]]
    wait_for_turn_completion: Callable[[str, str, int], Awaitable[TurnCompletion]]
    complete_job: Callable[[QueueJob], Awaitable[None]]
    flush_jobs: Callable[[QueueJob, str | None], Awaitable[list[QueueJobSummary]]]
    report_retry: Callable[[QueueJob, str], Awaitable[None]]
    report_batch_failure: Callable[[QueueJob, str, list[QueueJobSummary]], Awaitable[None]]
    log: Callable[[str], None]


def build_recovery_prompt(prompt: str, reason: str) -> str:
    return "\n".join(
        (
            "[Discord queue recovery attempt 2/2]",
            "The previous Codex turn did not complete successfully.",
            f"Previous outcome: {reason}",
            "Inspect the existing thread and workspace state first.",
            "Do not repeat side effects or steps that already completed.",
            "Continue the original request below:",
            "",
            prompt,
        )
    )


def format_turn_failure(completion: TurnCompletion) -> str:
    if completion.status is TurnStatus.FAILED:
        detail = completion.error_message.strip()
        return f"Codex turn failed: {detail or 'no error detail'}"
    if completion.status is TurnStatus.INTERRUPTED:
        origin = completion.interrupt_origin.value if completion.interrupt_origin else "external_or_unknown"
        return f"Codex turn was interrupted: {origin}"
    return f"Codex turn ended in unexpected state: {completion.status.value}"


async def process_queue_job(
    job: QueueJob,
    target_thread_id: str | None,
    *,
    deps: QueueTurnCoordinatorDeps,
) -> QueueProcessResult:
    original_prompt = str(job.get("prompt") or "").strip()
    prompt = original_prompt
    recovery = False
    while True:
        attempt = await deps.acquire_turn(
            job,
            prompt,
            target_thread_id,
            recovery=recovery,
        )
        deps.log(
            "queue_turn_owned "
            + f"job={job.get('job_id') or '-'} target={attempt.thread_id} "
            + f"turn={attempt.turn_id} attempt={attempt.attempt_number}"
        )
        completion = await deps.wait_for_turn_completion(
            attempt.thread_id,
            attempt.turn_id,
            attempt.app_server_generation,
        )
        if completion.status is TurnStatus.COMPLETED:
            await deps.complete_job(job)
            deps.log(
                "queue_turn_completed "
                + f"job={job.get('job_id') or '-'} target={attempt.thread_id} "
                + f"turn={attempt.turn_id} attempt={attempt.attempt_number}"
            )
            return QueueProcessResult.COMPLETED

        reason = format_turn_failure(completion)
        if attempt.attempt_number < 2:
            await deps.report_retry(job, reason)
            deps.log(
                "queue_turn_retry "
                + f"job={job.get('job_id') or '-'} target={attempt.thread_id} "
                + f"turn={attempt.turn_id} reason={reason[:300]}"
            )
            prompt = build_recovery_prompt(original_prompt, reason)
            recovery = True
            continue

        deleted_jobs = await deps.flush_jobs(job, target_thread_id)
        await deps.report_batch_failure(job, reason, deleted_jobs)
        deps.log(
            "queue_batch_flushed "
            + f"job={job.get('job_id') or '-'} target={attempt.thread_id} "
            + f"turn={attempt.turn_id} deleted={len(deleted_jobs)} reason={reason[:300]}"
        )
        return QueueProcessResult.FLUSHED
