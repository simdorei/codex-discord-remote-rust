"""Queue state helpers for per-thread Discord runner jobs."""

from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from collections.abc import Awaitable, Callable, Coroutine
from typing import NotRequired, Protocol, TypeAlias, TypedDict

from codex_discord_runtime import normalize_runner_key


class QueueChannelRef(Protocol):
    @property
    def id(self) -> int: ...


class QueueAuthorRef(Protocol):
    @property
    def id(self) -> int: ...


class QueueSourceMessageRef(Protocol):
    @property
    def author(self) -> QueueAuthorRef: ...


QueueJobValue: TypeAlias = QueueChannelRef | QueueSourceMessageRef | None


class QueueJob(TypedDict, total=False):
    job_id: str
    channel: QueueJobValue
    channel_id: int
    owner_user_id: int | None
    discord_message_id: int | None
    app_server_generation: int
    prompt: str
    target_thread_id: str | None
    queued: bool
    ack_sent: bool
    source_message: QueueJobValue
    state: str
    attempt_count: int
    turn_id: str | None
    baseline_turn_ids: tuple[str, ...]
    last_error: str


QueueItem: TypeAlias = QueueJob


class ThreadRunner(TypedDict):
    queue: asyncio.Queue[QueueItem]
    task: asyncio.Task[None] | None
    active: bool
    target_thread_id: str | None
    active_job_id: NotRequired[str | None]
    queued_job_ids: NotRequired[set[str]]


RunnerMap: TypeAlias = dict[str, ThreadRunner]
NormalizeRunnerKeyFunc: TypeAlias = Callable[[str | None], str]
ThreadRunnerLoopFunc: TypeAlias = Callable[[str | None], Coroutine[None, None, None]]
GetThreadRunnerFunc: TypeAlias = Callable[[str | None], Awaitable[ThreadRunner]]


THREAD_RUNNERS_LOCK = asyncio.Lock()
THREAD_RUNNERS: RunnerMap = {}


async def get_thread_runner(
    target_thread_id: str | None,
    *,
    runners: RunnerMap = THREAD_RUNNERS,
    runners_lock: asyncio.Lock = THREAD_RUNNERS_LOCK,
    normalize_runner_key_func: NormalizeRunnerKeyFunc = normalize_runner_key,
) -> ThreadRunner:
    key = normalize_runner_key_func(target_thread_id)
    async with runners_lock:
        runner = runners.get(key)
        if runner is not None:
            return runner
        new_runner: ThreadRunner = {
            "queue": asyncio.Queue[QueueItem](),
            "task": None,
            "active": False,
            "target_thread_id": target_thread_id,
            "active_job_id": None,
            "queued_job_ids": set(),
        }
        runners[key] = new_runner
        return new_runner


async def is_thread_runner_busy(
    target_thread_id: str | None,
    *,
    get_thread_runner_func: GetThreadRunnerFunc = get_thread_runner,
) -> bool:
    runner = await get_thread_runner_func(target_thread_id)
    return runner["active"] or runner["queue"].qsize() > 0


async def enqueue_thread_ask(
    channel: QueueJobValue,
    prompt: str,
    target_thread_id: str | None,
    *,
    queued: bool = False,
    ack_sent: bool = False,
    source_message: QueueJobValue = None,
    thread_runner_loop_func: ThreadRunnerLoopFunc,
    get_thread_runner_func: GetThreadRunnerFunc = get_thread_runner,
) -> int:
    job: QueueJob = {
        "channel": channel,
        "prompt": prompt,
        "target_thread_id": target_thread_id,
        "queued": queued,
        "ack_sent": ack_sent,
        "source_message": source_message,
    }
    return await enqueue_existing_thread_ask(
        job,
        target_thread_id,
        thread_runner_loop_func=thread_runner_loop_func,
        get_thread_runner_func=get_thread_runner_func,
    )


async def enqueue_existing_thread_ask(
    job: QueueJob,
    target_thread_id: str | None,
    *,
    thread_runner_loop_func: ThreadRunnerLoopFunc,
    get_thread_runner_func: GetThreadRunnerFunc = get_thread_runner,
) -> int:
    runner = await get_thread_runner_func(target_thread_id)
    queue = runner["queue"]
    job_id = str(job.get("job_id") or "").strip()
    queued_job_ids = runner.setdefault("queued_job_ids", set())
    if not job_id or (
        str(runner.get("active_job_id") or "") != job_id
        and job_id not in queued_job_ids
    ):
        await queue.put(job)
        if job_id:
            queued_job_ids.add(job_id)
    task = runner.get("task")
    if not isinstance(task, asyncio.Task) or task.done():
        runner["task"] = asyncio.create_task(thread_runner_loop_func(target_thread_id))
    return queue.qsize()


async def retract_thread_ask(
    target_thread_id: str | None,
    *,
    channel_id: int | None = None,
    owner_user_id: int | None = None,
    job_id: str | None = None,
    runners: RunnerMap = THREAD_RUNNERS,
    runners_lock: asyncio.Lock = THREAD_RUNNERS_LOCK,
    normalize_runner_key_func: NormalizeRunnerKeyFunc = normalize_runner_key,
) -> dict[str, int | bool | str]:
    key = normalize_runner_key_func(target_thread_id)

    def matches(job: QueueItem) -> bool:
        if job_id is not None and str(job.get("job_id") or "") != job_id:
            return False
        if channel_id is not None:
            channel = job.get("channel")
            stored_channel_id = _int_job_value(
                job.get("channel_id"),
                fallback=int(getattr(channel, "id", 0) or 0),
            )
            if stored_channel_id != int(channel_id):
                return False
        if owner_user_id is not None:
            source_message = job.get("source_message")
            author = getattr(source_message, "author", None)
            stored_owner_id = _int_job_value(
                job.get("owner_user_id"),
                fallback=int(getattr(author, "id", 0) or 0),
            )
            if stored_owner_id != int(owner_user_id):
                return False
        return True

    async with runners_lock:
        runner = runners.get(key)
        if runner is None:
            return {"removed": 0, "remaining": 0, "active": False, "target_key": key}
        queue = runner["queue"]

        drained: list[QueueItem] = []
        while True:
            try:
                drained.append(queue.get_nowait())
            except asyncio.QueueEmpty:
                break

        remove_index = -1
        for index, job in enumerate(drained):
            if matches(job):
                remove_index = index

        removed = 0
        for index, job in enumerate(drained):
            queue.task_done()
            if index == remove_index:
                runner.setdefault("queued_job_ids", set()).discard(str(job.get("job_id") or ""))
                removed += 1
                continue
            queue.put_nowait(job)

        return {
            "removed": removed,
            "remaining": queue.qsize(),
            "active": runner["active"],
            "target_key": key,
        }


def _int_job_value(value: int | None, *, fallback: int) -> int:
    return fallback if value is None else value
