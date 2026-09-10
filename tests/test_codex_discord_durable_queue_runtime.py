from __future__ import annotations

from collections.abc import Callable
from contextlib import contextmanager
from pathlib import Path
import sqlite3
import tempfile
from dataclasses import dataclass
from typing import cast
import unittest
from unittest import mock

from codex_app_server_transport_lifecycle import (
    AppServerGenerationMismatch,
    AppServerLifecycleSnapshot,
)
from codex_app_server_transport_turn_outcomes import (
    TurnCompletion,
    TurnCompletionPending,
    TurnStatus,
)
from codex_discord_durable_queue_runtime import (
    DurableQueueRuntime,
    DurableQueueRuntimeDeps,
)
from codex_discord_durable_queue_restore import QueueRestoreUnstableError
import codex_discord_prompt_delivery_prepare as prompt_delivery_prepare
import codex_discord_prompt_mapped_delivery as prompt_mapped_delivery
from codex_discord_queue_job_memory import to_memory_queue_job
from codex_discord_runner_queue import QueueJobValue
from codex_discord_queue_processor import (
    QueueGenerationExpiredError,
    QueueTurnOwnershipAmbiguousError,
)
import codex_discord_store as store
from codex_discord_store_queue import QueueEnqueueResult


@dataclass(frozen=True, slots=True)
class FakeQueueChannel:
    id: int

    async def send(self, _text: str) -> None:
        return None


@dataclass(frozen=True, slots=True)
class FakeSourceMessage:
    id: int
    created_at: float


class FakeRestoreBot:
    def __init__(self) -> None:
        self.channel = FakeQueueChannel(id=222)
        self.cache_reads = 0

    def get_cached_channel_or_thread(self, channel_id: int) -> tuple[QueueJobValue, str]:
        _ = channel_id
        self.cache_reads += 1
        return cast(QueueJobValue, self.channel), "cache"

    async def fetch_channel(self, channel_id: int) -> QueueJobValue:
        _ = channel_id
        return cast(QueueJobValue, self.channel)

    def is_allowed_message_channel(self, channel: QueueJobValue) -> bool:
        _ = channel
        return True


def _insert_unresolved_fork_handoff(
    db_path: Path,
    *,
    observed_target_thread_id: str | None,
) -> None:
    with sqlite3.connect(db_path) as connection:
        _ = connection.execute(
            "CREATE TABLE IF NOT EXISTS codex_thread_fork_handoffs ("
            "handoff_id TEXT PRIMARY KEY, source_thread_id TEXT NOT NULL UNIQUE, "
            "observed_target_thread_id TEXT, target_thread_id TEXT, "
            "last_fork_error TEXT NOT NULL DEFAULT '')"
        )
        _ = connection.execute(
            "INSERT INTO codex_thread_fork_handoffs "
            "(handoff_id, source_thread_id, observed_target_thread_id, "
            "target_thread_id, last_fork_error) VALUES (?, ?, ?, NULL, ?)",
            (
                "handoff-a",
                "thread-1",
                observed_target_thread_id,
                "app-server request thread/fork timed out after 10000 ms",
            ),
        )


class DurableQueueRuntimeTests(unittest.IsolatedAsyncioTestCase):
    async def test_retracting_last_queue_job_notifies_app_server_cleanup(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "mirror.sqlite"
            _ = self._enqueue(db_path, "job-1")
            work_change_events: list[str] = []
            runtime = self._runtime(
                db_path,
                states={},
                sent_prompts=[],
                work_change_events=work_change_events,
            )

            record = await runtime.retract_job(
                "thread-1",
                channel_id=222,
                owner_user_id=7,
            )

            self.assertIsNotNone(record)
            self.assertEqual(work_change_events, ["changed"])
            self.assertEqual(store.list_queue_jobs(db_path), [])

    async def test_enqueue_holds_app_server_admission_through_store(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "mirror.sqlite"
            admission_events: list[str] = []
            runtime = self._runtime(
                db_path,
                states={},
                sent_prompts=[],
                admission_events=admission_events,
            )
            original_enqueue = store.enqueue_queue_job

            def observed_enqueue(
                db_path_arg: Path,
                *,
                job_id: str,
                target_thread_id: str,
                channel_id: int,
                owner_user_id: int | None,
                discord_message_id: int | None,
                app_server_generation: int,
                prompt: str,
                queued: bool,
                ack_sent: bool,
                created_at: float | None = None,
            ) -> QueueEnqueueResult:
                admission_events.append("store")
                return original_enqueue(
                    db_path_arg,
                    job_id=job_id,
                    target_thread_id=target_thread_id,
                    channel_id=channel_id,
                    owner_user_id=owner_user_id,
                    discord_message_id=discord_message_id,
                    app_server_generation=app_server_generation,
                    prompt=prompt,
                    queued=queued,
                    ack_sent=ack_sent,
                    created_at=created_at,
                )

            with mock.patch.object(store, "enqueue_queue_job", observed_enqueue):
                _ = await runtime.enqueue(
                    cast(QueueJobValue, FakeQueueChannel(id=222)),
                    "request",
                    "thread-1",
                    queued=True,
                    ack_sent=True,
                    source_message=cast(
                        QueueJobValue,
                        FakeSourceMessage(id=999, created_at=2.0),
                    ),
                )

            self.assertEqual(admission_events, ["enter", "store", "exit"])

    async def test_starting_job_adopts_only_new_turn_after_restart_without_resending(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "mirror.sqlite"
            record = self._enqueue(db_path, "job-1")
            record = store.begin_queue_job_attempt(
                db_path,
                record.job_id,
                baseline_turn_ids=("turn-old",),
                app_server_generation=3,
            )
            sent_prompts: list[str] = []
            runtime = self._runtime(
                db_path,
                states={
                    "turn-old": self._turn("turn-old", TurnStatus.COMPLETED),
                    "turn-new": self._turn("turn-new", TurnStatus.IN_PROGRESS),
                },
                sent_prompts=sent_prompts,
            )
            channel = FakeQueueChannel(id=222)
            job = to_memory_queue_job(record, cast(QueueJobValue, channel), None)

            attempt = await runtime.acquire_turn(
                job,
                record.prompt,
                record.target_thread_id,
                recovery=False,
            )

            persisted = store.list_queue_jobs(db_path)[0]
            self.assertEqual(attempt.attempt_number, 1)
            self.assertEqual(attempt.turn_id, "turn-new")
            self.assertEqual(sent_prompts, [])
            self.assertEqual(persisted.turn_id, "turn-new")

    async def test_starting_job_with_no_new_turn_sends_same_prepared_attempt_once(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "mirror.sqlite"
            record = self._enqueue(db_path, "job-1")
            record = store.begin_queue_job_attempt(
                db_path,
                record.job_id,
                baseline_turn_ids=("turn-old",),
                app_server_generation=3,
            )
            sent_prompts: list[str] = []
            runtime = self._runtime(
                db_path,
                states={"turn-old": self._turn("turn-old", TurnStatus.COMPLETED)},
                sent_prompts=sent_prompts,
            )
            channel = FakeQueueChannel(id=222)
            job = to_memory_queue_job(record, cast(QueueJobValue, channel), None)

            attempt = await runtime.acquire_turn(
                job,
                record.prompt,
                record.target_thread_id,
                recovery=False,
            )

            persisted = store.list_queue_jobs(db_path)[0]
            self.assertEqual(attempt.attempt_number, 1)
            self.assertEqual(sent_prompts, ["request"])
            self.assertEqual(persisted.attempt_count, 1)
            self.assertEqual(persisted.turn_id, "turn-sent")

    async def test_immediate_job_preserves_non_queued_start_ack(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "mirror.sqlite"
            record = self._enqueue(db_path, "job-1", queued=False, ack_sent=False)
            prompt_kwargs: list[dict[str, object]] = []
            runtime = self._runtime(
                db_path,
                states={},
                sent_prompts=[],
                prompt_kwargs=prompt_kwargs,
            )
            channel = FakeQueueChannel(id=222)
            job = to_memory_queue_job(record, cast(QueueJobValue, channel), None)

            _ = await runtime.acquire_turn(
                job,
                record.prompt,
                record.target_thread_id,
                recovery=False,
            )

            self.assertEqual(len(prompt_kwargs), 1)
            self.assertIs(prompt_kwargs[0]["queued"], False)
            self.assertIs(prompt_kwargs[0]["ack_sent"], False)

    async def test_pending_job_waits_for_existing_in_progress_turn_before_sending(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "mirror.sqlite"
            record = self._enqueue(db_path, "job-1")
            sent_prompts: list[str] = []
            state_reads = 0

            def get_states(_thread_id: str) -> dict[str, TurnCompletion]:
                nonlocal state_reads
                state_reads += 1
                status = TurnStatus.IN_PROGRESS if state_reads == 1 else TurnStatus.COMPLETED
                return {"turn-existing": self._turn("turn-existing", status)}

            runtime = self._runtime(
                db_path,
                states={},
                sent_prompts=sent_prompts,
                get_states=get_states,
            )
            channel = FakeQueueChannel(id=222)
            job = to_memory_queue_job(record, cast(QueueJobValue, channel), None)

            attempt = await runtime.acquire_turn(
                job,
                record.prompt,
                record.target_thread_id,
                recovery=False,
            )

            self.assertEqual(state_reads, 2)
            self.assertEqual(sent_prompts, ["request"])
            self.assertEqual(attempt.turn_id, "turn-sent")

    async def test_enqueue_persists_current_app_server_generation(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "mirror.sqlite"
            runtime = self._runtime(db_path, states={}, sent_prompts=[])
            channel = FakeQueueChannel(id=222)

            job, created, position = await runtime.enqueue(
                cast(QueueJobValue, channel),
                "request",
                "thread-1",
                queued=True,
                ack_sent=True,
                source_message=None,
            )

            records = store.list_queue_jobs(db_path)

        self.assertTrue(created)
        self.assertEqual(position, 1)
        self.assertEqual(job.get("app_server_generation"), 3)
        self.assertEqual(records[0].app_server_generation, 3)

    async def test_enqueue_rejects_source_message_from_before_accepting_since(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "mirror.sqlite"
            runtime = self._runtime(db_path, states={}, sent_prompts=[])
            channel = FakeQueueChannel(id=222)
            source = FakeSourceMessage(id=999, created_at=0.5)

            with self.assertRaises(QueueGenerationExpiredError):
                _ = await runtime.enqueue(
                    cast(QueueJobValue, channel),
                    "request",
                    "thread-1",
                    queued=True,
                    ack_sent=True,
                    source_message=cast(QueueJobValue, source),
                )

            self.assertEqual(store.list_queue_jobs(db_path), [])

    async def test_enqueue_rejects_generation_changed_after_prompt_admission(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "mirror.sqlite"
            runtime = self._runtime(
                db_path,
                states={},
                sent_prompts=[],
                lifecycle=lambda: AppServerLifecycleSnapshot(4, True, 2.0),
                expected_generation=3,
            )
            channel = FakeQueueChannel(id=222)

            with self.assertRaises(QueueGenerationExpiredError):
                _ = await runtime.enqueue(
                    cast(QueueJobValue, channel),
                    "request",
                    "thread-1",
                    queued=True,
                    ack_sent=True,
                    source_message=None,
                )

            self.assertEqual(store.list_queue_jobs(db_path), [])

    async def test_stale_cleanup_never_deletes_job_inserted_by_new_generation(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "mirror.sqlite"
            _ = store.enqueue_queue_job(
                db_path,
                job_id="job-old",
                target_thread_id="thread-1",
                channel_id=222,
                owner_user_id=7,
                discord_message_id=998,
                app_server_generation=2,
                prompt="old",
                queued=True,
                ack_sent=True,
            )
            lifecycle_calls = 0

            def lifecycle() -> AppServerLifecycleSnapshot:
                nonlocal lifecycle_calls
                lifecycle_calls += 1
                if lifecycle_calls == 1:
                    return AppServerLifecycleSnapshot(3, True, 1.0)
                if lifecycle_calls == 2:
                    _ = store.enqueue_queue_job(
                        db_path,
                        job_id="job-new",
                        target_thread_id="thread-1",
                        channel_id=222,
                        owner_user_id=7,
                        discord_message_id=999,
                        app_server_generation=4,
                        prompt="new",
                        queued=True,
                        ack_sent=True,
                    )
                return AppServerLifecycleSnapshot(4, True, 2.0)

            runtime = self._runtime(
                db_path,
                states={},
                sent_prompts=[],
                lifecycle=lifecycle,
                expected_generation=3,
            )

            with self.assertRaises(QueueGenerationExpiredError):
                _ = await runtime.enqueue(
                    cast(QueueJobValue, FakeQueueChannel(id=222)),
                    "request",
                    "thread-1",
                    queued=True,
                    ack_sent=True,
                    source_message=None,
                )

            records = store.list_queue_jobs(db_path)

        self.assertEqual([record.job_id for record in records], ["job-new"])
        self.assertEqual(records[0].app_server_generation, 4)

    async def test_unhealthy_enqueue_discards_existing_jobs_without_storing_new_job(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "mirror.sqlite"
            _ = self._enqueue(db_path, "job-old")
            runtime = self._runtime(
                db_path,
                states={},
                sent_prompts=[],
                lifecycle=lambda: AppServerLifecycleSnapshot(3, False, None),
            )
            channel = FakeQueueChannel(id=222)

            with self.assertRaises(QueueGenerationExpiredError):
                _ = await runtime.enqueue(
                    cast(QueueJobValue, channel),
                    "new request",
                    "thread-1",
                    queued=True,
                    ack_sent=True,
                    source_message=None,
                )

            self.assertEqual(store.list_queue_jobs(db_path), [])

    async def test_stale_job_is_deleted_before_turn_delivery(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "mirror.sqlite"
            record = self._enqueue(db_path, "job-1")
            sent_prompts: list[str] = []
            runtime = self._runtime(
                db_path,
                states={},
                sent_prompts=sent_prompts,
                lifecycle=lambda: AppServerLifecycleSnapshot(4, True, 2.0),
            )
            job = to_memory_queue_job(
                record,
                cast(QueueJobValue, FakeQueueChannel(id=222)),
                None,
            )

            with self.assertRaises(QueueGenerationExpiredError):
                _ = await runtime.acquire_turn(
                    job,
                    record.prompt,
                    record.target_thread_id,
                    recovery=False,
                )

            self.assertEqual(sent_prompts, [])
            self.assertEqual(store.list_queue_jobs(db_path), [])

    async def test_generation_change_while_waiting_discards_job_without_retry(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "mirror.sqlite"
            _ = self._enqueue(db_path, "job-1")
            snapshot_reads = 0

            def lifecycle() -> AppServerLifecycleSnapshot:
                nonlocal snapshot_reads
                snapshot_reads += 1
                if snapshot_reads == 1:
                    return AppServerLifecycleSnapshot(3, True, 1.0)
                return AppServerLifecycleSnapshot(3, False, None)

            runtime = self._runtime(
                db_path,
                states={},
                sent_prompts=[],
                lifecycle=lifecycle,
            )

            with self.assertRaises(QueueGenerationExpiredError):
                _ = await runtime.wait_for_turn_completion("thread-1", "turn-1", 3)

            self.assertEqual(store.list_queue_jobs(db_path), [])

    async def test_restore_adopts_all_persisted_jobs_into_current_generation(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "mirror.sqlite"
            _ = store.enqueue_queue_job(
                db_path,
                job_id="job-stale",
                target_thread_id="thread-1",
                channel_id=222,
                owner_user_id=7,
                discord_message_id=998,
                app_server_generation=2,
                prompt="stale",
                queued=True,
                ack_sent=True,
            )
            _ = self._enqueue(db_path, "job-current")
            runtime = self._runtime(db_path, states={}, sent_prompts=[])
            bot = FakeRestoreBot()

            jobs = await runtime.restore_jobs(bot)
            records = store.list_queue_jobs(db_path)

        self.assertCountEqual(
            [job.get("job_id") for job in jobs],
            ["job-stale", "job-current"],
        )
        self.assertCountEqual(
            [record.job_id for record in records],
            ["job-stale", "job-current"],
        )
        self.assertTrue(all(record.app_server_generation == 3 for record in records))
        self.assertEqual(bot.cache_reads, 2)

    async def test_direct_rust_quarantine_job_refuses_poll_or_replay(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "mirror.sqlite"
            _ = self._enqueue(db_path, "quarantined")
            _ = store.begin_queue_job_attempt(
                db_path,
                "quarantined",
                baseline_turn_ids=(),
                app_server_generation=3,
            )
            _ = store.mark_queue_job_running(
                db_path,
                "quarantined",
                "cdr-quarantined:handoff-a",
                app_server_generation=3,
            )
            with sqlite3.connect(db_path) as connection:
                _ = connection.execute(
                    "UPDATE codex_turn_queue SET last_error = ? WHERE job_id = ?",
                    (
                        "[cdr-rust:app-server-fork-quarantine:v1] active writer",
                        "quarantined",
                    ),
                )
            record = store.list_queue_jobs(db_path)[0]
            job = to_memory_queue_job(
                record,
                cast(QueueJobValue, FakeQueueChannel(id=222)),
                None,
            )
            state_reads: list[str] = []
            sent_prompts: list[str] = []
            runtime = self._runtime(
                db_path,
                states={},
                sent_prompts=sent_prompts,
                get_states=lambda thread_id: state_reads.append(thread_id) or {},
            )

            with self.assertRaisesRegex(
                QueueTurnOwnershipAmbiguousError,
                "quarantined",
            ):
                _ = await runtime.acquire_turn(
                    job,
                    record.prompt,
                    record.target_thread_id,
                    recovery=False,
                )

            self.assertEqual(state_reads, [])
            self.assertEqual(sent_prompts, [])

    async def test_direct_unresolved_rust_fork_job_refuses_poll_or_replay(self) -> None:
        for observed_target in (None, "fork-created-before-crash"):
            with self.subTest(observed_target=observed_target):
                with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
                    db_path = Path(temp_dir) / "mirror.sqlite"
                    _ = self._enqueue(db_path, "fork-fenced")
                    _insert_unresolved_fork_handoff(
                        db_path,
                        observed_target_thread_id=observed_target,
                    )
                    record = store.list_queue_jobs(db_path)[0]
                    job = to_memory_queue_job(
                        record,
                        cast(QueueJobValue, FakeQueueChannel(id=222)),
                        None,
                    )
                    state_reads: list[str] = []
                    sent_prompts: list[str] = []
                    runtime = self._runtime(
                        db_path,
                        states={},
                        sent_prompts=sent_prompts,
                        get_states=lambda thread_id: state_reads.append(thread_id) or {},
                    )

                    for recovery in (False, True):
                        with self.subTest(recovery=recovery):
                            with self.assertRaisesRegex(
                                QueueTurnOwnershipAmbiguousError,
                                "app-server request thread/fork timed out after 10000 ms",
                            ):
                                _ = await runtime.acquire_turn(
                                    job,
                                    record.prompt,
                                    record.target_thread_id,
                                    recovery=recovery,
                                )

                    self.assertEqual(state_reads, [])
                    self.assertEqual(sent_prompts, [])

    async def test_restore_while_unhealthy_surfaces_failure_without_resolving_channels(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "mirror.sqlite"
            _ = self._enqueue(db_path, "job-1")
            runtime = self._runtime(
                db_path,
                states={},
                sent_prompts=[],
                lifecycle=lambda: AppServerLifecycleSnapshot(3, False, None),
            )
            bot = FakeRestoreBot()

            with self.assertRaises(QueueRestoreUnstableError):
                _ = await runtime.restore_jobs(bot)

            self.assertEqual(
                [record.job_id for record in store.list_queue_jobs(db_path)],
                ["job-1"],
            )
            self.assertEqual(bot.cache_reads, 0)

    def _runtime(
        self,
        db_path: Path,
        *,
        states: dict[str, TurnCompletion],
        sent_prompts: list[str],
        get_states: Callable[[str], dict[str, TurnCompletion]] | None = None,
        lifecycle: Callable[[], AppServerLifecycleSnapshot] | None = None,
        expected_generation: int | None = None,
        prompt_error: AppServerGenerationMismatch | None = None,
        prompt_kwargs: list[dict[str, object]] | None = None,
        admission_events: list[str] | None = None,
        work_change_events: list[str] | None = None,
    ) -> DurableQueueRuntime:
        async def run_prompt(
            channel: QueueJobValue,
            prompt: str,
            **kwargs: QueueJobValue,
        ) -> prompt_delivery_prepare.PromptDeliveryPreparationResult:
            _ = channel
            if prompt_error is not None:
                raise prompt_error
            sent_prompts.append(prompt)
            if prompt_kwargs is not None:
                prompt_kwargs.append(dict(kwargs))
            return prompt_delivery_prepare.PromptDeliveryPreparationResult(
                handled=True,
                target_thread_id="thread-1",
                target_ref="thread-1",
                recent_offsets={},
                delegate_to_session_mirror=False,
                mapped_result=prompt_mapped_delivery.MappedPromptDeliveryResult(
                    handled=True,
                    accepted=True,
                    turn_id="turn-sent",
                ),
            )

        async def send_chunks(
            target: QueueJobValue,
            text: str,
            **_kwargs: QueueJobValue,
        ) -> int:
            _ = target, text
            return 1

        lifecycle_getter = lifecycle or (lambda: AppServerLifecycleSnapshot(3, True, 1.0))

        @contextmanager
        def admit(_expected: int | None):
            if admission_events is not None:
                admission_events.append("enter")
            try:
                yield lifecycle_getter()
            finally:
                if admission_events is not None:
                    admission_events.append("exit")

        return DurableQueueRuntime(
            DurableQueueRuntimeDeps(
                get_db_path=lambda: db_path,
                get_app_server_lifecycle=lifecycle_getter,
                ensure_app_server_ready=lambda: None,
                get_expected_app_server_generation=lambda: expected_generation,
                admit_app_server_generation=admit,
                notify_app_server_work_changed=(
                    (lambda: work_change_events.append("changed"))
                    if work_change_events is not None
                    else (lambda: None)
                ),
                get_turn_states=(
                    (lambda thread_id, _generation: get_states(thread_id))
                    if get_states is not None
                    else (lambda _thread_id, _generation: states)
                ),
                wait_for_live_turn=lambda _thread_id, _turn_id, _timeout, _generation: TurnCompletionPending(),
                run_prompt_and_send=run_prompt,
                send_chunks=send_chunks,
                log=lambda _message: None,
            )
        )

    @staticmethod
    def _enqueue(
        db_path: Path,
        job_id: str,
        *,
        discord_message_id: int = 999,
        queued: bool = True,
        ack_sent: bool = True,
    ):
        return store.enqueue_queue_job(
            db_path,
            job_id=job_id,
            target_thread_id="thread-1",
            channel_id=222,
            owner_user_id=7,
            discord_message_id=discord_message_id,
            app_server_generation=3,
            prompt="request",
            queued=queued,
            ack_sent=ack_sent,
            created_at=1.0,
        ).job

    @staticmethod
    def _turn(turn_id: str, status: TurnStatus) -> TurnCompletion:
        return TurnCompletion("thread-1", turn_id, status)


if __name__ == "__main__":
    _ = unittest.main()
