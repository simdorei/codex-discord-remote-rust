from __future__ import annotations

from concurrent.futures import ThreadPoolExecutor
import tempfile
import unittest
from pathlib import Path
import sqlite3
from threading import Event

import codex_discord_store as store
from codex_discord_store_queue import QueueJobNotFoundError, QueueJobState, StoredQueueJob


def _insert_unresolved_fork_handoff(
    db_path: Path,
    *,
    handoff_id: str,
    source_thread_id: str,
    observed_target_thread_id: str | None,
    last_fork_error: str = "app-server request thread/fork timed out after 10000 ms",
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
                handoff_id,
                source_thread_id,
                observed_target_thread_id,
                last_fork_error,
            ),
        )


def _insert_completed_unmapped_fork_handoff(
    db_path: Path,
    *,
    source_thread_id: str,
    target_thread_id: str,
) -> None:
    with sqlite3.connect(db_path) as connection:
        _ = connection.execute(
            "CREATE TABLE codex_thread_fork_handoffs ("
            "handoff_id TEXT PRIMARY KEY, ambiguous_job_id TEXT UNIQUE, "
            "source_thread_id TEXT NOT NULL UNIQUE, expected_generation INTEGER NOT NULL, "
            "discord_channel_id INTEGER NOT NULL, discord_thread_id INTEGER NOT NULL, "
            "quarantine_reason TEXT NOT NULL, last_fork_error TEXT NOT NULL DEFAULT '', "
            "fork_failure_ambiguous INTEGER NOT NULL DEFAULT 0, "
            "observed_target_thread_id TEXT, target_thread_id TEXT UNIQUE, "
            "completed_generation INTEGER, created_at REAL NOT NULL, completed_at REAL, "
            "CHECK ((target_thread_id IS NULL AND completed_generation IS NULL "
            "AND completed_at IS NULL) OR (target_thread_id IS NOT NULL "
            "AND completed_generation IS NOT NULL AND completed_at IS NOT NULL)), "
            "CHECK (target_thread_id IS NULL OR "
            "target_thread_id = observed_target_thread_id))"
        )
        _ = connection.execute(
            "INSERT INTO codex_thread_fork_handoffs "
            "(handoff_id, ambiguous_job_id, source_thread_id, expected_generation, "
            "discord_channel_id, discord_thread_id, quarantine_reason, "
            "observed_target_thread_id, target_thread_id, completed_generation, "
            "created_at, completed_at) VALUES (?, NULL, ?, 4, 0, 0, ?, ?, ?, 4, 10, 11)",
            (
                "handoff-completed",
                source_thread_id,
                "unmapped selected target handoff",
                target_thread_id,
                target_thread_id,
            ),
        )


class QueueStoreTests(unittest.TestCase):
    def test_enqueue_refuses_completed_unmapped_rust_fork_source_without_inserting(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "mirror.sqlite"
            _ = store.enqueue_queue_job(
                db_path,
                job_id="existing",
                target_thread_id="bot-owned",
                channel_id=222,
                owner_user_id=7,
                discord_message_id=998,
                app_server_generation=4,
                prompt="existing",
                queued=True,
                ack_sent=True,
            )
            _insert_completed_unmapped_fork_handoff(
                db_path,
                source_thread_id="desktop-old",
                target_thread_id="rust-managed-new",
            )

            with self.assertRaisesRegex(
                store.QueueTargetMovedError,
                "rust-managed-new",
            ) as raised:
                _ = store.enqueue_queue_job(
                    db_path,
                    job_id="must-not-exist",
                    target_thread_id="desktop-old",
                    channel_id=222,
                    owner_user_id=7,
                    discord_message_id=999,
                    app_server_generation=4,
                    prompt="must not be inserted",
                    queued=True,
                    ack_sent=True,
                )

            records = store.list_queue_jobs(db_path)

        self.assertEqual(raised.exception.source_thread_id, "desktop-old")
        self.assertEqual(raised.exception.target_thread_id, "rust-managed-new")
        self.assertEqual([record.job_id for record in records], ["existing"])

    def test_enqueue_refuses_each_unresolved_rust_fork_source_without_inserting(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "mirror.sqlite"
            _ = store.enqueue_queue_job(
                db_path,
                job_id="existing",
                target_thread_id="bot-owned",
                channel_id=222,
                owner_user_id=7,
                discord_message_id=998,
                app_server_generation=4,
                prompt="existing",
                queued=True,
                ack_sent=True,
            )
            cases = (
                (
                    "desktop-unobserved",
                    None,
                    "app-server request thread/fork timed out after 10000 ms",
                ),
                (
                    "desktop-staged",
                    "fork-created-before-crash",
                    "fork target was persisted but local finalization failed",
                ),
            )
            for index, (source, observed_target, exact_error) in enumerate(cases):
                _insert_unresolved_fork_handoff(
                    db_path,
                    handoff_id=f"handoff-{index}",
                    source_thread_id=source,
                    observed_target_thread_id=observed_target,
                    last_fork_error=exact_error,
                )
                with self.subTest(source=source, observed_target=observed_target):
                    with self.assertRaisesRegex(
                        store.QueueTargetForkFencedError,
                        exact_error,
                    ):
                        _ = store.enqueue_queue_job(
                            db_path,
                            job_id=f"blocked-{index}",
                            target_thread_id=source,
                            channel_id=222,
                            owner_user_id=7,
                            discord_message_id=1_000 + index,
                            app_server_generation=4,
                            prompt="must not be inserted",
                            queued=True,
                            ack_sent=True,
                        )

            records = store.list_queue_jobs(db_path)

        self.assertEqual([record.job_id for record in records], ["existing"])

    def test_rust_quarantine_sentinel_survives_every_python_replay_mutation(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "mirror.sqlite"
            _ = store.enqueue_queue_job(
                db_path,
                job_id="quarantined",
                target_thread_id="desktop-owned",
                channel_id=222,
                owner_user_id=7,
                discord_message_id=999,
                app_server_generation=2,
                prompt="must never replay",
                queued=True,
                ack_sent=True,
            )
            _ = store.begin_queue_job_attempt(
                db_path,
                "quarantined",
                baseline_turn_ids=("old-turn",),
                app_server_generation=2,
            )
            _ = store.mark_queue_job_running(
                db_path,
                "quarantined",
                "cdr-quarantined:handoff-a",
                app_server_generation=2,
            )
            with sqlite3.connect(db_path) as connection:
                _ = connection.execute(
                    "UPDATE codex_turn_queue SET last_error = ? WHERE job_id = ?",
                    (
                        "[cdr-rust:app-server-fork-quarantine:v1] active writer",
                        "quarantined",
                    ),
                )
            _ = store.enqueue_queue_job(
                db_path,
                job_id="ordinary",
                target_thread_id="bot-owned",
                channel_id=222,
                owner_user_id=7,
                discord_message_id=1_000,
                app_server_generation=2,
                prompt="ordinary",
                queued=True,
                ack_sent=True,
            )

            adoption = store.adopt_queue_jobs_generation(db_path, 4)
            sentinel = next(
                record
                for record in store.list_queue_jobs(db_path)
                if record.job_id == "quarantined"
            )
            stale = store.discard_queue_jobs_for_generation(db_path, 4)
            observed = store.discard_observed_queue_jobs(db_path, [sentinel])
            flushed = store.flush_queue_jobs(
                db_path,
                "desktop-owned",
                app_server_generation=2,
            )
            completed = store.complete_queue_job(db_path, "quarantined")
            with self.assertRaises(QueueJobNotFoundError):
                _ = store.begin_queue_job_attempt(
                    db_path,
                    "quarantined",
                    baseline_turn_ids=(),
                    app_server_generation=2,
                )
            with self.assertRaises(QueueJobNotFoundError):
                _ = store.mark_queue_job_running(
                    db_path,
                    "quarantined",
                    "must-not-replace",
                    app_server_generation=2,
                )
            self.assertIsNone(
                store.retract_queue_job(
                    db_path,
                    "desktop-owned",
                    channel_id=222,
                    owner_user_id=7,
                )
            )
            unhealthy = store.discard_queue_jobs_for_generation(db_path, None)
            duplicate = store.enqueue_queue_job(
                db_path,
                job_id="duplicate",
                target_thread_id="bot-owned",
                channel_id=222,
                owner_user_id=7,
                discord_message_id=999,
                app_server_generation=4,
                prompt="duplicate must resolve to sentinel",
                queued=True,
                ack_sent=True,
            )
            remaining = store.list_queue_jobs(db_path)
            has_replayable = store.has_replayable_queue_jobs(db_path)

        self.assertEqual([record.job_id for record in adoption.jobs], ["ordinary"])
        self.assertEqual(adoption.quarantined_count, 1)
        self.assertEqual(stale, [])
        self.assertEqual(observed, [])
        self.assertEqual(flushed, [])
        self.assertFalse(completed)
        self.assertEqual([record.job_id for record in unhealthy], ["ordinary"])
        self.assertFalse(duplicate.created)
        self.assertEqual(duplicate.job.job_id, "quarantined")
        self.assertEqual([record.job_id for record in remaining], ["quarantined"])
        self.assertEqual(remaining[0].app_server_generation, 2)
        self.assertEqual(remaining[0], sentinel)
        self.assertFalse(has_replayable)

    def test_unresolved_rust_fork_sources_survive_every_python_replay_mutation(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "mirror.sqlite"
            for job_id, source in (
                ("fork-unobserved", "desktop-unobserved"),
                ("fork-staged", "desktop-staged"),
            ):
                _ = store.enqueue_queue_job(
                    db_path,
                    job_id=job_id,
                    target_thread_id=source,
                    channel_id=222,
                    owner_user_id=7,
                    discord_message_id=None,
                    app_server_generation=2,
                    prompt=f"must never replay {job_id}",
                    queued=True,
                    ack_sent=True,
                )
            _insert_unresolved_fork_handoff(
                db_path,
                handoff_id="handoff-unobserved",
                source_thread_id="desktop-unobserved",
                observed_target_thread_id=None,
            )
            _insert_unresolved_fork_handoff(
                db_path,
                handoff_id="handoff-staged",
                source_thread_id="desktop-staged",
                observed_target_thread_id="fork-created-before-crash",
            )
            _ = store.enqueue_queue_job(
                db_path,
                job_id="ordinary",
                target_thread_id="bot-owned",
                channel_id=222,
                owner_user_id=7,
                discord_message_id=None,
                app_server_generation=2,
                prompt="ordinary",
                queued=True,
                ack_sent=True,
            )
            original = {
                record.job_id: record
                for record in store.list_queue_jobs(db_path)
                if record.job_id.startswith("fork-")
            }

            adoption = store.adopt_queue_jobs_generation(db_path, 4)
            replayable_before_cleanup = store.has_replayable_queue_jobs(db_path)
            stale = store.discard_queue_jobs_for_generation(db_path, 4)
            observed = store.discard_observed_queue_jobs(db_path, tuple(original.values()))
            flushed = [
                *store.flush_queue_jobs(
                    db_path,
                    "desktop-unobserved",
                    app_server_generation=2,
                ),
                *store.flush_queue_jobs(
                    db_path,
                    "desktop-staged",
                    app_server_generation=2,
                ),
            ]
            completed = [
                store.complete_queue_job(db_path, job_id)
                for job_id in original
            ]
            for job_id in original:
                with self.assertRaises(QueueJobNotFoundError):
                    _ = store.begin_queue_job_attempt(
                        db_path,
                        job_id,
                        baseline_turn_ids=(),
                        app_server_generation=2,
                    )
                with self.assertRaises(QueueJobNotFoundError):
                    _ = store.mark_queue_job_running(
                        db_path,
                        job_id,
                        "must-not-replace",
                        app_server_generation=2,
                    )
            retracted = [
                store.retract_queue_job(
                    db_path,
                    source,
                    channel_id=222,
                    owner_user_id=7,
                )
                for source in ("desktop-unobserved", "desktop-staged")
            ]
            unhealthy = store.discard_queue_jobs_for_generation(db_path, None)
            replayable_after_cleanup = store.has_replayable_queue_jobs(db_path)
            remaining = {
                record.job_id: record for record in store.list_queue_jobs(db_path)
            }

        self.assertEqual([record.job_id for record in adoption.jobs], ["ordinary"])
        self.assertEqual(adoption.fork_fenced_count, 2)
        self.assertTrue(replayable_before_cleanup)
        self.assertEqual(stale, [])
        self.assertEqual(observed, [])
        self.assertEqual(flushed, [])
        self.assertEqual(completed, [False, False])
        self.assertEqual(retracted, [None, None])
        self.assertEqual([record.job_id for record in unhealthy], ["ordinary"])
        self.assertFalse(replayable_after_cleanup)
        self.assertEqual(remaining, original)

    def test_queue_job_survives_reopen_and_duplicate_discord_message_is_idempotent(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "mirror.sqlite"
            first = store.enqueue_queue_job(
                db_path,
                job_id="job-1",
                target_thread_id="thread-1",
                channel_id=222,
                owner_user_id=7,
                discord_message_id=999,
                app_server_generation=4,
                prompt="first request",
                queued=True,
                ack_sent=True,
                created_at=10.0,
            )
            duplicate = store.enqueue_queue_job(
                db_path,
                job_id="job-duplicate",
                target_thread_id="thread-1",
                channel_id=222,
                owner_user_id=7,
                discord_message_id=999,
                app_server_generation=4,
                prompt="first request",
                queued=True,
                ack_sent=True,
                created_at=11.0,
            )

            records = store.list_queue_jobs(db_path)

        self.assertTrue(first.created)
        self.assertFalse(duplicate.created)
        self.assertEqual(duplicate.job.job_id, "job-1")
        self.assertEqual([record.job_id for record in records], ["job-1"])
        self.assertIs(records[0].state, QueueJobState.PENDING)
        self.assertEqual(records[0].app_server_generation, 4)

    def test_attempt_turn_and_flush_state_are_durable_and_target_scoped(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "mirror.sqlite"
            for index, target in enumerate(("thread-1", "thread-1", "thread-2"), start=1):
                _ = store.enqueue_queue_job(
                    db_path,
                    job_id=f"job-{index}",
                    target_thread_id=target,
                    channel_id=200 + index,
                    owner_user_id=7,
                    discord_message_id=900 + index,
                    app_server_generation=4,
                    prompt=f"request {index}",
                    queued=True,
                    ack_sent=True,
                    created_at=float(index),
                )
            started = store.begin_queue_job_attempt(
                db_path,
                "job-1",
                baseline_turn_ids=("turn-old",),
                app_server_generation=4,
            )
            running = store.mark_queue_job_running(
                db_path,
                "job-1",
                "turn-new",
                app_server_generation=4,
            )

            deleted = store.flush_queue_jobs(
                db_path,
                "thread-1",
                app_server_generation=4,
            )
            remaining = store.list_queue_jobs(db_path)

        self.assertEqual(started.attempt_count, 1)
        self.assertIs(started.state, QueueJobState.STARTING)
        self.assertEqual(started.baseline_turn_ids, ("turn-old",))
        self.assertIs(running.state, QueueJobState.RUNNING)
        self.assertEqual(running.turn_id, "turn-new")
        self.assertEqual([record.job_id for record in deleted], ["job-1", "job-2"])
        self.assertEqual([record.job_id for record in remaining], ["job-3"])

    def test_legacy_queue_schema_migrates_existing_rows_to_stale_generation_zero(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "mirror.sqlite"
            with sqlite3.connect(db_path) as conn:
                _ = conn.execute(
                    "CREATE TABLE codex_turn_queue ("
                    "job_id TEXT PRIMARY KEY, target_thread_id TEXT NOT NULL, "
                    "channel_id INTEGER NOT NULL, owner_user_id INTEGER, "
                    "discord_message_id INTEGER, prompt TEXT NOT NULL, queued INTEGER NOT NULL, "
                    "ack_sent INTEGER NOT NULL, state TEXT NOT NULL, attempt_count INTEGER NOT NULL, "
                    "turn_id TEXT, baseline_turn_ids TEXT NOT NULL, last_error TEXT NOT NULL DEFAULT '', "
                    "created_at REAL NOT NULL, updated_at REAL NOT NULL)"
                )
                _ = conn.execute(
                    "INSERT INTO codex_turn_queue VALUES "
                    "('legacy', 'thread-1', 222, NULL, NULL, 'old', 1, 1, "
                    "'pending', 0, NULL, '[]', '', 1.0, 1.0)"
                )

            store.init_mirror_db(db_path)
            records = store.list_queue_jobs(db_path)
            with sqlite3.connect(db_path) as conn:
                columns = {
                    str(row[1])
                    for row in conn.execute("PRAGMA table_info(codex_turn_queue)").fetchall()
                }

        self.assertIn("app_server_generation", columns)
        self.assertEqual(records[0].app_server_generation, 0)

    def test_generation_discard_atomically_returns_deleted_rows(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "mirror.sqlite"
            for generation in (3, 4):
                _ = store.enqueue_queue_job(
                    db_path,
                    job_id=f"job-{generation}",
                    target_thread_id="thread-1",
                    channel_id=222,
                    owner_user_id=7,
                    discord_message_id=900 + generation,
                    app_server_generation=generation,
                    prompt=f"request {generation}",
                    queued=True,
                    ack_sent=True,
                )

            stale = store.discard_queue_jobs_for_generation(db_path, 4)
            current = store.list_queue_jobs(db_path)
            unhealthy = store.discard_queue_jobs_for_generation(db_path, None)
            empty = store.list_queue_jobs(db_path)

        self.assertEqual([record.job_id for record in stale], ["job-3"])
        self.assertEqual([record.job_id for record in current], ["job-4"])
        self.assertEqual([record.job_id for record in unhealthy], ["job-4"])
        self.assertEqual(empty, [])

    def test_observed_discard_does_not_delete_rows_outside_observed_set(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "mirror.sqlite"
            for job_id, generation in (("job-old", 3), ("job-new", 4)):
                _ = store.enqueue_queue_job(
                    db_path,
                    job_id=job_id,
                    target_thread_id="thread-1",
                    channel_id=222,
                    owner_user_id=7,
                    discord_message_id=None,
                    app_server_generation=generation,
                    prompt=job_id,
                    queued=True,
                    ack_sent=False,
                )

            observed = [
                record
                for record in store.list_queue_jobs(db_path)
                if record.job_id == "job-old"
            ]
            deleted = store.discard_observed_queue_jobs(db_path, observed)
            remaining = store.list_queue_jobs(db_path)

        self.assertEqual([record.job_id for record in deleted], ["job-old"])
        self.assertEqual([record.job_id for record in remaining], ["job-new"])

    def test_observed_discard_preserves_job_readopted_before_delete(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "mirror.sqlite"
            _ = store.enqueue_queue_job(
                db_path,
                job_id="job-recovered",
                target_thread_id="thread-1",
                channel_id=222,
                owner_user_id=7,
                discord_message_id=999,
                app_server_generation=2,
                prompt="recover me",
                queued=True,
                ack_sent=True,
            )
            observation_ready = Event()
            adoption_done = Event()

            # Given: stale cleanup has observed generation 2 but has not deleted it.
            def cleanup_after_barrier() -> list[StoredQueueJob]:
                observed = store.list_queue_jobs(db_path)
                observation_ready.set()
                if not adoption_done.wait(timeout=5):
                    self.fail("generation adoption did not reach the cleanup barrier")
                return store.discard_observed_queue_jobs(db_path, observed)

            with ThreadPoolExecutor(max_workers=1) as executor:
                cleanup = executor.submit(cleanup_after_barrier)
                self.assertTrue(observation_ready.wait(timeout=5))

                # When: recovery adopts the same job before stale cleanup deletes it.
                _ = store.adopt_queue_jobs_generation(db_path, 4)
                adoption_done.set()
                deleted = cleanup.result(timeout=5)

            remaining = store.list_queue_jobs(db_path)

        # Then: the old observation cannot delete the newly adopted row.
        self.assertEqual(deleted, [])
        self.assertEqual([record.job_id for record in remaining], ["job-recovered"])
        self.assertEqual(remaining[0].app_server_generation, 4)


if __name__ == "__main__":
    _ = unittest.main()
