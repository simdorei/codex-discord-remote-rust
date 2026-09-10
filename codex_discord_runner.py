"""Runner queue helpers for Discord ask delivery."""

from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
import time
import traceback
from collections.abc import Awaitable, Callable
from typing import TypeAlias

from codex_discord_queue_processor import (
    QueueGenerationExpiredError,
    QueueProcessResult,
    QueueTurnCoordinatorDeps,
    QueueTurnOwnershipAmbiguousError,
    process_queue_job,
)
from codex_discord_runtime import normalize_runner_key
from codex_discord_runner_queue import (
    THREAD_RUNNERS,
    THREAD_RUNNERS_LOCK,
    QueueItem,
    QueueJob,
    GetThreadRunnerFunc,
    NormalizeRunnerKeyFunc,
    RunnerMap,
    ThreadRunner,
    enqueue_existing_thread_ask,
    enqueue_thread_ask,
    get_thread_runner,
    is_thread_runner_busy,
    retract_thread_ask,
)

BusyState: TypeAlias = tuple[str, str | None, str]
GetBusyStateFunc: TypeAlias = Callable[[str | None], BusyState]
MonotonicFunc: TypeAlias = Callable[[], float]
SleepFunc: TypeAlias = Callable[[float], Awaitable[None]]
ToThreadBusyStateFunc: TypeAlias = Callable[[GetBusyStateFunc, str | None], Awaitable[BusyState]]
SendTextFunc: TypeAlias = Callable[..., Awaitable[int]]
LogFunc: TypeAlias = Callable[[str], None]
WaitForIdleFunc: TypeAlias = Callable[[str | None], Awaitable[BusyState]]
ReportJobFailedFunc: TypeAlias = Callable[[QueueJob, str | None], Awaitable[None]]

__all__ = (
    "THREAD_RUNNERS",
    "THREAD_RUNNERS_LOCK",
    "enqueue_existing_thread_ask",
    "enqueue_thread_ask",
    "get_thread_runner",
    "is_thread_runner_busy",
    "retract_thread_ask",
    "wait_for_codex_thread_idle",
    "report_thread_runner_job_failed",
    "thread_runner_loop",
)


async def _to_thread_busy_state(
    func: GetBusyStateFunc,
    target_thread_id: str | None,
) -> BusyState:
    return await asyncio.to_thread(func, target_thread_id)


async def wait_for_codex_thread_idle(
    target_thread_id: str | None,
    *,
    get_busy_state_func: GetBusyStateFunc,
    timeout_sec: float = 3600.0,
    poll_sec: float = 5.0,
    monotonic_func: MonotonicFunc = time.monotonic,
    sleep_func: SleepFunc = asyncio.sleep,
    to_thread_func: ToThreadBusyStateFunc = _to_thread_busy_state,
) -> BusyState:
    deadline = monotonic_func() + timeout_sec
    last_state = "idle"
    last_thread_id: str | None = None
    last_ref = ""
    while monotonic_func() < deadline:
        state, resolved_thread_id, target_ref = await to_thread_func(
            get_busy_state_func,
            target_thread_id,
        )
        last_state = state
        last_thread_id = resolved_thread_id
        last_ref = target_ref
        if state == "idle":
            return state, resolved_thread_id, target_ref
        await sleep_func(poll_sec)
    return last_state, last_thread_id, last_ref


async def report_thread_runner_job_failed(
    job: QueueJob,
    target_thread_id: str | None,
    *,
    send_text_func: SendTextFunc,
    log_func: LogFunc,
) -> None:
    channel = job.get("channel")
    if channel is None or not hasattr(channel, "send"):
        return
    try:
        _ = await send_text_func(
            channel,
            "Queued ask failed. Check codex_discord_bot.log.",
            context="thread_runner_job_failed",
        )
        log_func(f"thread_runner_job_failure_reported target={target_thread_id or '-'}")
    except Exception:  # noqa: BROAD_EXCEPT_OK
        log_func("thread_runner_job_failure_report_failed\n" + traceback.format_exc())


async def thread_runner_loop(
    target_thread_id: str | None,
    *,
    get_busy_state_func: GetBusyStateFunc,
    wait_for_idle_func: WaitForIdleFunc,
    queue_coordinator_deps: QueueTurnCoordinatorDeps,
    report_job_failed_func: ReportJobFailedFunc,
    send_text_func: SendTextFunc,
    log_func: LogFunc,
    runners: RunnerMap = THREAD_RUNNERS,
    runners_lock: asyncio.Lock = THREAD_RUNNERS_LOCK,
    normalize_runner_key_func: NormalizeRunnerKeyFunc = normalize_runner_key,
    get_thread_runner_func: GetThreadRunnerFunc = get_thread_runner,
    to_thread_func: ToThreadBusyStateFunc = _to_thread_busy_state,
) -> None:
    key = normalize_runner_key_func(target_thread_id)
    while True:
        runner = await get_thread_runner_func(target_thread_id)
        queue = runner["queue"]
        try:
            job = await asyncio.wait_for(queue.get(), timeout=5)
            job_for_failure = job
        except asyncio.TimeoutError:
            async with runners_lock:
                current = runners.get(key)
                if current is None or current is not runner:
                    continue
                if not current["active"] and queue.empty():
                    _ = runners.pop(key, None)
                    return
            continue

        runner["active"] = True
        active_job_id = str(job.get("job_id") or "")
        runner["active_job_id"] = active_job_id or None
        if active_job_id:
            runner.setdefault("queued_job_ids", set()).discard(active_job_id)
        try:
            channel = job.get("channel")
            prompt = str(job.get("prompt") or "").strip()
            job_target_thread_id = str(job.get("target_thread_id") or "").strip() or None
            if prompt and hasattr(channel, "send"):
                queued = bool(job.get("queued"))
                if queued and not job.get("job_id"):
                    busy_state, _busy_thread_id, busy_ref = await to_thread_func(
                        get_busy_state_func,
                        job_target_thread_id,
                    )
                    if busy_state != "idle":
                        busy_label = busy_ref or job_target_thread_id or "selected"
                        _ = await send_text_func(
                            channel,
                            f"Queued ask waiting for `{busy_label}` to become idle. Current state: {busy_state}.",
                            context="thread_runner_waiting",
                        )
                        busy_state, _busy_thread_id, busy_ref = await wait_for_idle_func(
                            job_target_thread_id,
                        )
                        if busy_state != "idle":
                            busy_label = busy_ref or job_target_thread_id or "selected"
                            _ = await send_text_func(
                                channel,
                                f"Queued ask is still blocked for `{busy_label}`. Current state: {busy_state}.",
                                context="thread_runner_still_blocked",
                            )
                            continue
                result = await process_queue_job(
                    job,
                    job_target_thread_id,
                    deps=queue_coordinator_deps,
                )
                if result is QueueProcessResult.FLUSHED:
                    _drain_pending_queue(queue, runner)
        except QueueGenerationExpiredError as exc:
            current_generation = exc.current_generation if exc.healthy else None
            discarded = _drain_generation_expired_queue(
                queue,
                runner,
                current_generation=current_generation,
            )
            log_func(
                f"queue_generation_discarded target={target_thread_id or '-'} "
                f"expected={exc.expected_generation} "
                f"current={current_generation if current_generation is not None else 'unhealthy'} "
                f"memory_discarded={discarded}"
            )
        except QueueTurnOwnershipAmbiguousError as exc:
            deleted_jobs = await queue_coordinator_deps.flush_jobs(job_for_failure, target_thread_id)
            await queue_coordinator_deps.report_batch_failure(job_for_failure, str(exc), deleted_jobs)
            _drain_pending_queue(queue, runner)
            log_func(
                f"queue_batch_flushed_ambiguous target={target_thread_id or '-'} "
                + f"deleted={len(deleted_jobs)} error={str(exc)[:300]}"
            )
        except Exception:  # noqa: BROAD_EXCEPT_OK
            log_func("thread_runner_job_failed\n" + traceback.format_exc())
            await report_job_failed_func(job_for_failure, target_thread_id)
            if job_for_failure.get("job_id"):
                await asyncio.sleep(2.0)
                await queue.put(job_for_failure)
                runner.setdefault("queued_job_ids", set()).add(str(job_for_failure.get("job_id") or ""))
        finally:
            runner["active"] = False
            runner["active_job_id"] = None
            queue.task_done()


def _drain_pending_queue(
    queue: asyncio.Queue[QueueItem],
    runner: ThreadRunner,
) -> None:
    queued_job_ids = runner.setdefault("queued_job_ids", set())
    while True:
        try:
            pending = queue.get_nowait()
        except asyncio.QueueEmpty:
            return
        job_id = str(pending.get("job_id") or "")
        if job_id:
            queued_job_ids.discard(job_id)
        queue.task_done()


def _drain_generation_expired_queue(
    queue: asyncio.Queue[QueueItem],
    runner: ThreadRunner,
    *,
    current_generation: int | None,
) -> int:
    queued_job_ids = runner.setdefault("queued_job_ids", set())
    retained: list[QueueItem] = []
    discarded = 0
    while True:
        try:
            pending = queue.get_nowait()
        except asyncio.QueueEmpty:
            break
        queue.task_done()
        job_id = str(pending.get("job_id") or "")
        if job_id:
            queued_job_ids.discard(job_id)
        generation = int(pending.get("app_server_generation") or 0)
        if current_generation is not None and generation == current_generation:
            retained.append(pending)
        else:
            discarded += 1
    for pending in retained:
        queue.put_nowait(pending)
        job_id = str(pending.get("job_id") or "")
        if job_id:
            queued_job_ids.add(job_id)
    return discarded
