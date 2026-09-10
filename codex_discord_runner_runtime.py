"""Runtime wiring for Discord runner queues."""

from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from collections.abc import AsyncGenerator, Awaitable, Callable
from contextlib import asynccontextmanager
from dataclasses import dataclass
from typing import TypeAlias

import codex_discord_queue_messages as discord_queue_messages
import codex_discord_queue_targets as discord_queue_targets
import codex_discord_durable_queue_runtime as durable_queue_runtime
import codex_discord_runner as discord_runner
import codex_discord_runtime as discord_runtime
from codex_discord_runner_queue import QueueJobValue, RunnerMap, ThreadRunner
from codex_discord_runner_queue import QueueJob
from codex_discord_durable_queue_restore import QueueRestoreBot


QueueRetractResult: TypeAlias = dict[str, int | bool | str]
BuildRunnersMessageFunc: TypeAlias = Callable[[], Awaitable[str]]
FormatTargetRefForLogFunc: TypeAlias = Callable[[str], str]
GetQueueTargetBridgeFunc: TypeAlias = Callable[[], discord_queue_targets.QueueTargetBridge]
SnapshotThreadRunnersFunc: TypeAlias = Callable[[], dict[str, discord_runtime.RunnerState]]


@dataclass(frozen=True, slots=True)
class RunnerRuntimeDeps:
    thread_runners: RunnerMap
    thread_runners_lock: asyncio.Lock
    runner_snapshot_lock: discord_runtime.RunnerLockLike
    snapshot_thread_runners: SnapshotThreadRunnersFunc
    get_runtime_state: Callable[[], discord_runtime.DiscordRuntimeState]
    get_busy_state_for_thread: discord_runner.GetBusyStateFunc
    resolve_target_ref: discord_runtime.ResolveTargetRefFunc
    get_queue_target_bridge: GetQueueTargetBridgeFunc
    get_mirrored_codex_thread_id: discord_queue_targets.GetMirroredCodexThreadIdFunc
    format_target_ref_for_log: FormatTargetRefForLogFunc
    durable_queue: durable_queue_runtime.DurableQueueRuntime
    send_chunks: discord_runner.SendTextFunc
    log: discord_runner.LogFunc


@dataclass(frozen=True, slots=True)
class RunnerRuntime:
    deps: RunnerRuntimeDeps

    async def build_runners_message(self) -> str:
        async with self.deps.thread_runners_lock:
            runners_snapshot = self.deps.snapshot_thread_runners()
        return await discord_runtime.build_runners_message(
            runners_snapshot,
            self.deps.runner_snapshot_lock,
            resolve_target_ref_func=self.deps.resolve_target_ref,
        )

    def resolve_queue_command_target(
        self,
        channel_id: int | None,
        ref: str | None,
    ) -> tuple[str | None, str]:
        return discord_queue_targets.resolve_queue_command_target(
            channel_id,
            ref,
            bridge_module=self.deps.get_queue_target_bridge(),
            resolve_target_ref_func=self.deps.resolve_target_ref,
            get_mirrored_codex_thread_id_func=self.deps.get_mirrored_codex_thread_id,
        )

    async def retract_queued_ask_for_request(
        self,
        *,
        channel_id: int | None,
        user_id: int | None,
        ref: str | None,
    ) -> tuple[str, QueueRetractResult]:
        target_thread_id, target_ref = self.resolve_queue_command_target(channel_id, ref)
        result = await self.retract_thread_ask(
            target_thread_id,
            channel_id=channel_id,
            owner_user_id=user_id,
        )
        self.deps.log(
            " ".join(
                [
                    f"queue_retract user={user_id or '-'}",
                    f"target={target_thread_id or '-'}",
                    f"target_ref={self.deps.format_target_ref_for_log(target_ref)}",
                    f"removed={int(result.get('removed') or 0)}",
                    f"remaining={int(result.get('remaining') or 0)}",
                    f"active={bool(result.get('active'))}",
                ]
            )
        )
        return discord_queue_messages.build_retract_message(result, target_ref), result

    @asynccontextmanager
    async def codex_app_turn_slot(self, target_thread_id: str | None) -> AsyncGenerator[bool]:
        async with discord_runtime.codex_app_turn_slot(
            self.deps.get_runtime_state(),
            target_thread_id,
            log=self.deps.log,
        ) as waited:
            yield waited

    async def get_thread_runner(self, target_thread_id: str | None) -> ThreadRunner:
        return await discord_runner.get_thread_runner(
            target_thread_id,
            runners=self.deps.thread_runners,
            runners_lock=self.deps.thread_runners_lock,
            normalize_runner_key_func=discord_runtime.normalize_runner_key,
        )

    async def wait_for_codex_thread_idle(
        self,
        target_thread_id: str | None,
        *,
        timeout_sec: float = 3600.0,
        poll_sec: float = 5.0,
    ) -> tuple[str, str | None, str]:
        return await discord_runner.wait_for_codex_thread_idle(
            target_thread_id,
            get_busy_state_func=self.deps.get_busy_state_for_thread,
            timeout_sec=timeout_sec,
            poll_sec=poll_sec,
        )

    async def enqueue_thread_ask(
        self,
        channel: QueueJobValue,
        prompt: str,
        target_thread_id: str | None,
        *,
        queued: bool = False,
        ack_sent: bool = False,
        source_message: QueueJobValue = None,
    ) -> int:
        if target_thread_id:
            job, _created, position = await self.deps.durable_queue.enqueue(
                channel,
                prompt,
                target_thread_id,
                queued=queued,
                ack_sent=ack_sent,
                source_message=source_message,
            )
            _ = await discord_runner.enqueue_existing_thread_ask(
                job,
                target_thread_id,
                get_thread_runner_func=self.get_thread_runner,
                thread_runner_loop_func=self.thread_runner_loop,
            )
            return position
        return await discord_runner.enqueue_thread_ask(
            channel,
            prompt,
            target_thread_id,
            queued=queued,
            ack_sent=ack_sent,
            source_message=source_message,
            get_thread_runner_func=self.get_thread_runner,
            thread_runner_loop_func=self.thread_runner_loop,
        )

    async def retract_thread_ask(
        self,
        target_thread_id: str | None,
        *,
        channel_id: int | None = None,
        owner_user_id: int | None = None,
    ) -> QueueRetractResult:
        if target_thread_id:
            record = await self.deps.durable_queue.retract_job(
                target_thread_id,
                channel_id=channel_id,
                owner_user_id=owner_user_id,
            )
            if record is not None:
                result = await discord_runner.retract_thread_ask(
                    target_thread_id,
                    job_id=record.job_id,
                    runners=self.deps.thread_runners,
                    runners_lock=self.deps.thread_runners_lock,
                    normalize_runner_key_func=discord_runtime.normalize_runner_key,
                )
                result["removed"] = 1
                return result
        return await discord_runner.retract_thread_ask(
            target_thread_id,
            channel_id=channel_id,
            owner_user_id=owner_user_id,
            runners=self.deps.thread_runners,
            runners_lock=self.deps.thread_runners_lock,
            normalize_runner_key_func=discord_runtime.normalize_runner_key,
        )

    async def report_thread_runner_job_failed(
        self,
        job: QueueJob,
        target_thread_id: str | None,
    ) -> None:
        await discord_runner.report_thread_runner_job_failed(
            job,
            target_thread_id,
            send_text_func=self.deps.send_chunks,
            log_func=self.deps.log,
        )

    async def thread_runner_loop(self, target_thread_id: str | None) -> None:
        await discord_runner.thread_runner_loop(
            target_thread_id,
            runners=self.deps.thread_runners,
            runners_lock=self.deps.thread_runners_lock,
            normalize_runner_key_func=discord_runtime.normalize_runner_key,
            get_thread_runner_func=self.get_thread_runner,
            get_busy_state_func=self.deps.get_busy_state_for_thread,
            wait_for_idle_func=self.wait_for_codex_thread_idle,
            queue_coordinator_deps=self.deps.durable_queue.coordinator_deps(),
            report_job_failed_func=self.report_thread_runner_job_failed,
            send_text_func=self.deps.send_chunks,
            log_func=self.deps.log,
        )

    async def restore_durable_queue_runners(
        self,
        bot: QueueRestoreBot,
    ) -> int:
        jobs = await self.deps.durable_queue.restore_jobs(bot)
        for job in jobs:
            target_thread_id = str(job.get("target_thread_id") or "").strip() or None
            _ = await discord_runner.enqueue_existing_thread_ask(
                job,
                target_thread_id,
                get_thread_runner_func=self.get_thread_runner,
                thread_runner_loop_func=self.thread_runner_loop,
            )
        self.deps.log(f"queue_restore_done jobs={len(jobs)}")
        return len(jobs)
