from __future__ import annotations

from collections.abc import Callable
from contextlib import contextmanager
from pathlib import Path
import sqlite3
import tempfile
from typing import cast
import unittest

from codex_app_server_transport_lifecycle import AppServerLifecycleSnapshot
from codex_app_server_transport_turn_outcomes import TurnCompletion, TurnStatus
from codex_discord_durable_queue_restore import restore_queue_jobs
from codex_discord_durable_queue_runtime import DurableQueueRuntime, DurableQueueRuntimeDeps
from codex_discord_queue_job_memory import to_memory_queue_job
from codex_discord_queue_processor import QueueTurnOwnershipAmbiguousError
from codex_discord_runner_queue import QueueJobValue
import codex_discord_store as store
from codex_discord_store_queue import QueueJobNotFoundError


HOLD_PREFIX = "[cdr-rust:turn-start-candidates-ambiguous:v1] "


class _Channel:
    id = 222

    async def send(self, _text: str) -> None:
        return None


class _Bot:
    def __init__(self) -> None:
        self.channel = _Channel()
        self.cache_reads = 0

    def get_cached_channel_or_thread(self, _channel_id: int) -> tuple[QueueJobValue, str]:
        self.cache_reads += 1
        return cast(QueueJobValue, self.channel), "cache"

    async def fetch_channel(self, _channel_id: int) -> QueueJobValue:
        return cast(QueueJobValue, self.channel)

    def is_allowed_message_channel(self, _channel: QueueJobValue) -> bool:
        return True


class StartingHoldRollbackTests(unittest.IsolatedAsyncioTestCase):
    def test_new_enqueue_is_rejected_for_held_target_but_allowed_elsewhere(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "queue.db"
            _seed_hold(db_path)
            before = store.list_queue_jobs(db_path)

            with self.assertRaisesRegex(
                store.QueueTargetStartingCandidatesHeldError,
                "held.*manual resolution",
            ):
                _enqueue(db_path, "must-not-insert", "held-thread", 2.0)
            self.assertEqual(store.list_queue_jobs(db_path), before)

            _enqueue(db_path, "other", "other-thread", 3.0)
            self.assertEqual(
                [job.job_id for job in store.list_queue_jobs(db_path)],
                ["held", "other"],
            )

    def test_python_store_never_mutates_or_replays_a_held_target(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "queue.db"
            _ = _seed_hold(db_path, following_id="following")
            _enqueue(db_path, "ordinary", "other-thread", 3.0)
            before = {job.job_id: job for job in store.list_queue_jobs(db_path)}

            adoption = store.adopt_queue_jobs_generation(db_path, 9)
            self.assertEqual([job.job_id for job in adoption.jobs], ["ordinary"])
            self.assertEqual(adoption.starting_held_count, 1)
            self.assertTrue(store.has_replayable_queue_jobs(db_path))
            self.assertEqual(store.discard_observed_queue_jobs(db_path, tuple(before.values())), [])
            self.assertEqual(store.flush_queue_jobs(db_path, "held-thread", app_server_generation=3), [])
            self.assertFalse(store.complete_queue_job(db_path, "held"))
            self.assertIsNone(
                store.retract_queue_job(
                    db_path,
                    "held-thread",
                    channel_id=222,
                    owner_user_id=7,
                )
            )
            with self.assertRaises(QueueJobNotFoundError):
                _ = store.begin_queue_job_attempt(
                    db_path,
                    "held",
                    baseline_turn_ids=(),
                    app_server_generation=3,
                )
            with self.assertRaises(QueueJobNotFoundError):
                _ = store.mark_queue_job_running(
                    db_path,
                    "held",
                    "must-not-adopt",
                    app_server_generation=3,
                )
            _ = store.discard_queue_jobs_for_generation(db_path, None)
            remaining = {job.job_id: job for job in store.list_queue_jobs(db_path)}

            self.assertEqual(remaining, {key: before[key] for key in ("held", "following")})
            self.assertFalse(store.has_replayable_queue_jobs(db_path))

    async def test_restore_skips_the_held_target_and_logs_the_reason(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "queue.db"
            _ = _seed_hold(db_path, following_id="following")
            _enqueue(db_path, "ordinary", "other-thread", 3.0)
            logs: list[str] = []
            snapshot = AppServerLifecycleSnapshot(9, True, 1.0)
            deps = _RestoreDeps(db_path, lambda: snapshot, logs)
            bot = _Bot()

            jobs = await restore_queue_jobs(bot, deps)
            records = {job.job_id: job for job in store.list_queue_jobs(db_path)}

            self.assertEqual([job.get("job_id") for job in jobs], ["ordinary"])
            self.assertEqual(records["held"].app_server_generation, 3)
            self.assertEqual(records["following"].app_server_generation, 3)
            self.assertEqual(records["ordinary"].app_server_generation, 9)
            self.assertEqual(bot.cache_reads, 1)
            self.assertTrue(any("queue_restore_starting_candidates_held skipped=1" in log for log in logs))

    async def test_zero_one_or_two_observed_turns_never_poll_replay_or_mutate_hold(self) -> None:
        for candidate_count in range(3):
            with self.subTest(candidate_count=candidate_count):
                with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
                    db_path = Path(temp_dir) / "queue.db"
                    _ = _seed_hold(db_path, following_id="forced-pending")
                    held = next(
                        job
                        for job in store.list_queue_jobs(db_path)
                        if job.job_id == "forced-pending"
                    )
                    before = store.list_queue_jobs(db_path)
                    reads: list[str] = []
                    prompts: list[str] = []
                    states = {
                        f"candidate-{index}": TurnCompletion(
                            "held-thread",
                            f"candidate-{index}",
                            TurnStatus.COMPLETED,
                        )
                        for index in range(candidate_count)
                    }
                    runtime = _runtime(db_path, states, reads, prompts)
                    job = to_memory_queue_job(held, cast(QueueJobValue, _Channel()), None)

                    for recovery in (False, True):
                        with self.assertRaisesRegex(
                            QueueTurnOwnershipAmbiguousError,
                            "held.*manual resolution",
                        ):
                            _ = await runtime.acquire_turn(
                                job,
                                held.prompt,
                                held.target_thread_id,
                                recovery=recovery,
                            )

                    self.assertEqual(reads, [])
                    self.assertEqual(prompts, [])
                    self.assertEqual(store.list_queue_jobs(db_path), before)


class _RestoreDeps:
    def __init__(
        self,
        db_path: Path,
        lifecycle: Callable[[], AppServerLifecycleSnapshot],
        logs: list[str],
    ) -> None:
        self.get_db_path = lambda: db_path
        self.get_app_server_lifecycle = lifecycle
        self.ensure_app_server_ready = lambda: None
        self.log = logs.append


def _runtime(
    db_path: Path,
    states: dict[str, TurnCompletion],
    reads: list[str],
    prompts: list[str],
) -> DurableQueueRuntime:
    lifecycle = lambda: AppServerLifecycleSnapshot(3, True, 1.0)

    @contextmanager
    def admit(_expected: int | None):
        yield lifecycle()

    async def run_prompt(_channel: QueueJobValue, prompt: str, **_kwargs: QueueJobValue):
        prompts.append(prompt)
        raise AssertionError("held prompt must never be replayed")

    return DurableQueueRuntime(
        DurableQueueRuntimeDeps(
            get_db_path=lambda: db_path,
            get_app_server_lifecycle=lifecycle,
            ensure_app_server_ready=lambda: None,
            get_expected_app_server_generation=lambda: 3,
            admit_app_server_generation=admit,
            notify_app_server_work_changed=lambda: None,
            get_turn_states=lambda thread_id, _generation: reads.append(thread_id) or states,
            wait_for_live_turn=lambda *_args: None,
            run_prompt_and_send=run_prompt,
            send_chunks=lambda *_args, **_kwargs: None,
            log=lambda _message: None,
        )
    )


def _seed_hold(db_path: Path, following_id: str | None = None):
    _enqueue(db_path, "held", "held-thread", 1.0)
    if following_id is not None:
        _enqueue(db_path, following_id, "held-thread", 1.5)
    _ = store.begin_queue_job_attempt(
        db_path,
        "held",
        baseline_turn_ids=("baseline",),
        app_server_generation=3,
    )
    with sqlite3.connect(db_path) as connection:
        _ = connection.execute(
            "UPDATE codex_turn_queue SET last_error = ? WHERE job_id = 'held'",
            (f"{HOLD_PREFIX}candidate_count=2; candidate_turn_ids=[]",),
        )
    return next(job for job in store.list_queue_jobs(db_path) if job.job_id == "held")


def _enqueue(db_path: Path, job_id: str, target: str, created_at: float) -> None:
    _ = store.enqueue_queue_job(
        db_path,
        job_id=job_id,
        target_thread_id=target,
        channel_id=222,
        owner_user_id=7,
        discord_message_id=None,
        app_server_generation=3,
        prompt=job_id,
        queued=True,
        ack_sent=True,
        created_at=created_at,
    )


if __name__ == "__main__":
    unittest.main()
