from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from collections.abc import Awaitable, Callable
from contextlib import AbstractContextManager
from dataclasses import dataclass
from pathlib import Path
import uuid

from codex_app_server_transport_lifecycle import (
    AppServerGenerationMismatch,
    AppServerLifecycleSnapshot,
)
from codex_app_server_transport_turn_outcomes import (
    TurnCompletion,
    TurnCompletionFound,
    TurnCompletionObservation,
    TurnCompletionTransportError,
    TurnStatus,
)
import codex_discord_prompt_delivery_prepare as prompt_delivery_prepare
from codex_discord_durable_queue_restore import QueueRestoreBot, restore_queue_jobs
import codex_discord_queue_generation as queue_generation
from codex_discord_queue_job_memory import (
    copy_stored_queue_state,
    queue_job_baseline,
    queue_job_int,
    to_memory_queue_job,
)
from codex_discord_queue_reporting import (
    build_queue_batch_failure_message,
    build_queue_retry_message,
)
from codex_discord_queue_processor import (
    QueueAttempt,
    QueueJobSummary,
    QueueTurnCoordinatorDeps,
    QueueTurnOwnershipAmbiguousError,
)
from codex_discord_runner_queue import QueueJob, QueueJobValue
import codex_discord_store as store
from codex_discord_store_queue import (
    QueueJobState,
    StoredQueueJob,
    is_quarantined_queue_values,
    is_starting_candidate_hold_values,
)


TurnStateGetter = Callable[[str, int], dict[str, TurnCompletion]]
LiveTurnWaiter = Callable[[str, str, float, int], TurnCompletionObservation]
PromptSender = Callable[..., Awaitable[prompt_delivery_prepare.PromptDeliveryPreparationResult]]
ChunkSender = Callable[..., Awaitable[int]]
QueueAdmission = Callable[
    [int | None],
    AbstractContextManager[AppServerLifecycleSnapshot],
]


class QueueTurnDeliveryError(RuntimeError):
    pass


@dataclass(frozen=True, slots=True)
class DurableQueueRuntimeDeps:
    get_db_path: Callable[[], Path]
    get_app_server_lifecycle: queue_generation.LifecycleSnapshotGetter
    ensure_app_server_ready: Callable[[], None]
    get_expected_app_server_generation: Callable[[], int | None]
    admit_app_server_generation: QueueAdmission
    notify_app_server_work_changed: Callable[[], None]
    get_turn_states: TurnStateGetter
    wait_for_live_turn: LiveTurnWaiter
    run_prompt_and_send: PromptSender
    send_chunks: ChunkSender
    log: Callable[[str], None]


@dataclass(frozen=True, slots=True)
class DurableQueueRuntime:
    deps: DurableQueueRuntimeDeps

    async def enqueue(
        self,
        channel: QueueJobValue,
        prompt: str,
        target_thread_id: str,
        *,
        queued: bool,
        ack_sent: bool,
        source_message: QueueJobValue,
    ) -> tuple[QueueJob, bool, int]:
        channel_id = int(getattr(channel, "id", 0) or 0)
        author = getattr(source_message, "author", None)
        owner_user_id = int(getattr(author, "id", 0) or 0) or None
        discord_message_id = int(getattr(source_message, "id", 0) or 0) or None
        if channel_id <= 0:
            raise QueueTurnDeliveryError("Durable queue delivery requires a Discord channel id.")
        expected_generation = self.deps.get_expected_app_server_generation()
        with self.deps.admit_app_server_generation(expected_generation) as snapshot:
            generation = int(snapshot.generation if expected_generation is None else expected_generation)
            if (
                not snapshot.healthy
                or generation <= 0
                or int(snapshot.generation) != generation
            ):
                await queue_generation.require_queue_generation(
                    self.deps,
                    generation,
                    "enqueue_admission",
                )
            _ = await queue_generation.discard_stale_queue_jobs(self.deps, snapshot)
            queue_generation.reject_source_before_accepting_since(
                self.deps,
                source_message,
                snapshot,
                channel_id=channel_id,
            )
            await queue_generation.require_queue_generation(
                self.deps,
                generation,
                "enqueue_before_store",
            )
            result = await asyncio.to_thread(
                store.enqueue_queue_job,
                self.deps.get_db_path(),
                job_id=str(uuid.uuid4()),
                target_thread_id=target_thread_id,
                channel_id=channel_id,
                owner_user_id=owner_user_id,
                discord_message_id=discord_message_id,
                app_server_generation=generation,
                prompt=prompt,
                queued=queued,
                ack_sent=ack_sent,
            )
            await queue_generation.require_queue_generation(
                self.deps,
                generation,
                "enqueue_after_store",
            )
            records = await asyncio.to_thread(
                store.list_queue_jobs,
                self.deps.get_db_path(),
                target_thread_id,
                app_server_generation=generation,
            )
            return to_memory_queue_job(result.job, channel, source_message), result.created, len(records)

    async def acquire_turn(
        self,
        job: QueueJob,
        prompt: str,
        target_thread_id: str | None,
        *,
        recovery: bool,
    ) -> QueueAttempt:
        job_id = str(job.get("job_id") or "").strip()
        thread_id = str(target_thread_id or job.get("target_thread_id") or "").strip()
        if not job_id or not thread_id:
            raise QueueTurnDeliveryError("Durable queue job is missing its job or thread id.")
        generation = queue_job_int(job.get("app_server_generation"))
        await self._refuse_protected_rust_job(job, thread_id)
        await queue_generation.require_queue_generation(self.deps, generation, "acquire")
        if not recovery:
            restored = await self._reconcile_restored_attempt(job, thread_id, generation)
            if restored is not None:
                return restored
        states = await self._wait_for_startable_turn(thread_id, generation)
        state = str(job.get("state") or QueueJobState.PENDING.value)
        attempt_count = queue_job_int(job.get("attempt_count"))
        if state != QueueJobState.STARTING.value or recovery:
            record = await asyncio.to_thread(
                store.begin_queue_job_attempt,
                self.deps.get_db_path(),
                job_id,
                baseline_turn_ids=tuple(states),
                app_server_generation=generation,
            )
            copy_stored_queue_state(job, record)
            attempt_count = record.attempt_count
        channel = job.get("channel")
        if channel is None or not hasattr(channel, "send"):
            raise QueueTurnDeliveryError("Durable queue job has no send-capable Discord channel.")
        await queue_generation.require_queue_generation(self.deps, generation, "turn_start")
        try:
            preparation = await self.deps.run_prompt_and_send(
                channel,
                prompt,
                queued=bool(job.get("queued")),
                ack_sent=bool(job.get("ack_sent")),
                source_message=job.get("source_message"),
                target_thread_id=thread_id,
                expected_app_server_generation=generation,
            )
        except AppServerGenerationMismatch as exc:
            await queue_generation.translate_transport_generation_expiry(
                self.deps,
                exc,
                generation,
                "turn_start",
                job_id=job_id,
            )
        mapped = preparation.mapped_result
        if mapped is None or not mapped.accepted or not mapped.turn_id:
            detail = mapped.error_message if mapped is not None else "mapped app-server delivery was not used"
            raise QueueTurnDeliveryError(f"Queue turn was not accepted: {detail[:500]}")
        await queue_generation.require_queue_generation(
            self.deps,
            generation,
            "turn_accepted",
        )
        record = await asyncio.to_thread(
            store.mark_queue_job_running,
            self.deps.get_db_path(),
            job_id,
            mapped.turn_id,
            app_server_generation=generation,
        )
        copy_stored_queue_state(job, record)
        return QueueAttempt(attempt_count, thread_id, mapped.turn_id, generation)

    async def _wait_for_startable_turn(
        self,
        thread_id: str,
        generation: int,
    ) -> dict[str, TurnCompletion]:
        waiting_logged = False
        while True:
            await queue_generation.require_queue_generation(
                self.deps,
                generation,
                "wait_for_startable_turn",
            )
            try:
                states = await asyncio.to_thread(
                    self.deps.get_turn_states,
                    thread_id,
                    generation,
                )
            except AppServerGenerationMismatch as exc:
                await queue_generation.translate_transport_generation_expiry(
                    self.deps,
                    exc,
                    generation,
                    "wait_for_startable_turn",
                )
            active_turn_ids = [
                turn_id
                for turn_id, completion in states.items()
                if completion.status is TurnStatus.IN_PROGRESS
            ]
            if not active_turn_ids:
                if waiting_logged:
                    self.deps.log(f"queue_active_turn_cleared target={thread_id}")
                return states
            if not waiting_logged:
                self.deps.log(
                    f"queue_waiting_for_active_turn target={thread_id} "
                    + f"turns={','.join(active_turn_ids[:3])}"
                )
                waiting_logged = True
            await asyncio.sleep(1.0)

    async def wait_for_turn_completion(
        self,
        thread_id: str,
        turn_id: str,
        generation: int,
    ) -> TurnCompletion:
        while True:
            await queue_generation.require_queue_generation(
                self.deps,
                generation,
                "wait_for_turn_completion",
            )
            try:
                observation = await asyncio.to_thread(
                    self.deps.wait_for_live_turn,
                    thread_id,
                    turn_id,
                    2.0,
                    generation,
                )
            except AppServerGenerationMismatch as exc:
                await queue_generation.translate_transport_generation_expiry(
                    self.deps,
                    exc,
                    generation,
                    "wait_for_turn_completion",
                )
            await queue_generation.require_queue_generation(
                self.deps,
                generation,
                "wait_for_turn_completion",
            )
            if isinstance(observation, TurnCompletionFound):
                return observation.completion
            if isinstance(observation, TurnCompletionTransportError):
                self.deps.log(
                    f"queue_turn_live_wait_transport_error target={thread_id} turn={turn_id} "
                    + f"error={observation.message[:300]}"
                )
            try:
                states = await asyncio.to_thread(
                    self.deps.get_turn_states,
                    thread_id,
                    generation,
                )
            except AppServerGenerationMismatch as exc:
                await queue_generation.translate_transport_generation_expiry(
                    self.deps,
                    exc,
                    generation,
                    "reconcile_turn_completion",
                )
            except (OSError, RuntimeError, TimeoutError) as exc:
                self.deps.log(
                    f"queue_turn_reconcile_retry target={thread_id} turn={turn_id} "
                    + f"error_type={type(exc).__name__} error={str(exc)[:300]}"
                )
                await asyncio.sleep(2.0)
                continue
            state = states.get(turn_id)
            if state is not None and state.status is not TurnStatus.IN_PROGRESS:
                return state
            await asyncio.sleep(1.0)

    async def complete_job(self, job: QueueJob) -> None:
        job_id = str(job.get("job_id") or "")
        generation = queue_job_int(job.get("app_server_generation"))
        await queue_generation.require_queue_generation(
            self.deps,
            generation,
            "complete_job",
        )
        _ = await asyncio.to_thread(store.complete_queue_job, self.deps.get_db_path(), job_id)
        self.deps.notify_app_server_work_changed()

    async def flush_jobs(self, job: QueueJob, target_thread_id: str | None) -> list[QueueJobSummary]:
        generation = queue_job_int(job.get("app_server_generation"))
        await queue_generation.require_queue_generation(self.deps, generation, "flush_jobs")
        thread_id = str(target_thread_id or job.get("target_thread_id") or "")
        records = await asyncio.to_thread(
            store.flush_queue_jobs,
            self.deps.get_db_path(),
            thread_id,
            app_server_generation=generation,
        )
        self.deps.notify_app_server_work_changed()
        return [QueueJobSummary(record.job_id, record.prompt) for record in records]

    async def report_retry(self, job: QueueJob, reason: str) -> None:
        generation = queue_job_int(job.get("app_server_generation"))
        await queue_generation.require_queue_generation(self.deps, generation, "report_retry")
        channel = job.get("channel")
        if channel is not None and hasattr(channel, "send"):
            _ = await self.deps.send_chunks(
                channel,
                build_queue_retry_message(reason),
                context="queue_turn_retry",
            )

    async def report_batch_failure(
        self,
        job: QueueJob,
        reason: str,
        deleted_jobs: list[QueueJobSummary],
    ) -> None:
        channel = job.get("channel")
        if channel is None or not hasattr(channel, "send"):
            return
        _ = await self.deps.send_chunks(
            channel,
            build_queue_batch_failure_message(reason, deleted_jobs),
            context="queue_batch_flushed",
        )

    def coordinator_deps(self) -> QueueTurnCoordinatorDeps:
        return QueueTurnCoordinatorDeps(
            acquire_turn=self.acquire_turn,
            wait_for_turn_completion=self.wait_for_turn_completion,
            complete_job=self.complete_job,
            flush_jobs=self.flush_jobs,
            report_retry=self.report_retry,
            report_batch_failure=self.report_batch_failure,
            log=self.deps.log,
        )

    async def restore_jobs(self, bot: QueueRestoreBot) -> list[QueueJob]:
        return await restore_queue_jobs(bot, self.deps)

    async def retract_job(
        self,
        target_thread_id: str,
        *,
        channel_id: int | None,
        owner_user_id: int | None,
    ) -> StoredQueueJob | None:
        record = await asyncio.to_thread(
            store.retract_queue_job,
            self.deps.get_db_path(),
            target_thread_id,
            channel_id=channel_id,
            owner_user_id=owner_user_id,
        )
        if record is not None:
            self.deps.notify_app_server_work_changed()
        return record

    async def _reconcile_restored_attempt(
        self,
        job: QueueJob,
        thread_id: str,
        generation: int,
    ) -> QueueAttempt | None:
        state = str(job.get("state") or QueueJobState.PENDING.value)
        attempt_count = queue_job_int(job.get("attempt_count"))
        turn_id = str(job.get("turn_id") or "").strip()
        if state == QueueJobState.RUNNING.value and turn_id:
            return QueueAttempt(attempt_count, thread_id, turn_id, generation)
        if state != QueueJobState.STARTING.value:
            return None
        baseline = queue_job_baseline(job)
        await queue_generation.require_queue_generation(
            self.deps,
            generation,
            "reconcile_restored_attempt",
        )
        try:
            states = await asyncio.to_thread(
                self.deps.get_turn_states,
                thread_id,
                generation,
            )
        except AppServerGenerationMismatch as exc:
            await queue_generation.translate_transport_generation_expiry(
                self.deps,
                exc,
                generation,
                "reconcile_restored_attempt",
            )
        candidates = [candidate for candidate in states if candidate not in baseline]
        if len(candidates) > 1:
            raise QueueTurnOwnershipAmbiguousError(
                f"Queue restart found multiple unowned turns for {thread_id}; refusing duplicate delivery."
            )
        if not candidates:
            return None
        turn_id = candidates[0]
        record = await asyncio.to_thread(
            store.mark_queue_job_running,
            self.deps.get_db_path(),
            str(job.get("job_id") or ""),
            turn_id,
            app_server_generation=generation,
        )
        copy_stored_queue_state(job, record)
        return QueueAttempt(record.attempt_count, thread_id, turn_id, generation)

    async def _refuse_protected_rust_job(
        self,
        job: QueueJob,
        thread_id: str,
    ) -> None:
        state = str(job.get("state") or QueueJobState.PENDING.value)
        turn_id = str(job.get("turn_id") or "").strip()
        last_error = str(job.get("last_error") or "")
        if is_starting_candidate_hold_values(state, turn_id or None, last_error):
            self._raise_starting_candidate_hold(thread_id, str(job.get("job_id") or "unknown"))
        if is_quarantined_queue_values(state, turn_id, last_error):
            raise QueueTurnOwnershipAmbiguousError(
                "Queue job is quarantined after an ambiguous Rust app-server start; "
                + "refusing to poll or replay it."
            )
        source_thread_id = str(job.get("target_thread_id") or thread_id).strip()
        held = await asyncio.to_thread(
            store.get_starting_candidate_hold_for_target,
            self.deps.get_db_path(),
            source_thread_id,
        )
        if held is not None:
            self._raise_starting_candidate_hold(source_thread_id, held.job_id)
        fence = await asyncio.to_thread(
            store.get_unresolved_app_server_fork_fence,
            self.deps.get_db_path(),
            source_thread_id,
        )
        if fence is None:
            return
        observed = (
            f" observed_target={fence.observed_target_thread_id};"
            if fence.observed_target_thread_id
            else ""
        )
        fork_error = fence.last_fork_error or last_error or "no fork error was recorded"
        raise QueueTurnOwnershipAmbiguousError(
            "Queue job is fenced by unresolved Rust app-server fork "
            + f"{fence.handoff_id} for {fence.source_thread_id};{observed} "
            + "refusing to poll or replay it; "
            + f"fork_error={fork_error}"
        )

    @staticmethod
    def _raise_starting_candidate_hold(thread_id: str, held_job_id: str) -> None:
        raise QueueTurnOwnershipAmbiguousError(
            "Queue target is held because Rust found multiple candidate turns and could not "
            + "safely identify the accepted turn; manual resolution is required. "
            + f"Refusing to poll or replay it: target={thread_id} held_job={held_job_id}"
        )
