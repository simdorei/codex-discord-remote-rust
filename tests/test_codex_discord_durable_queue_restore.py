from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
import sqlite3
import tempfile
import unittest

from codex_app_server_transport_lifecycle import AppServerLifecycleSnapshot
import codex_discord_durable_queue_restore as queue_restore
import codex_discord_store as store
from codex_discord_runner_queue import QueueJobValue


@dataclass(frozen=True, slots=True)
class _Channel:
    id: int


@dataclass(frozen=True, slots=True)
class _Deps:
    db_path: Path
    lifecycle: Callable[[], AppServerLifecycleSnapshot]
    logs: list[str]
    ensure_ready: Callable[[], None] = lambda: None

    @property
    def get_db_path(self) -> Callable[[], Path]:
        return lambda: self.db_path

    @property
    def get_app_server_lifecycle(self) -> Callable[[], AppServerLifecycleSnapshot]:
        return self.lifecycle

    @property
    def ensure_app_server_ready(self) -> Callable[[], None]:
        return self.ensure_ready

    @property
    def log(self) -> Callable[[str], None]:
        return self.logs.append


@dataclass(frozen=True, slots=True)
class _Bot:
    channel: _Channel

    def get_cached_channel_or_thread(self, channel_id: int) -> tuple[QueueJobValue, str]:
        return (self.channel, "cache") if channel_id == self.channel.id else (None, "miss")

    async def fetch_channel(self, channel_id: int) -> QueueJobValue:
        _ = channel_id
        return self.channel

    def is_allowed_message_channel(self, channel: QueueJobValue) -> bool:
        return channel is self.channel


def _insert_unresolved_fork_handoff(
    db_path: Path,
    *,
    handoff_id: str,
    source_thread_id: str,
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
                handoff_id,
                source_thread_id,
                observed_target_thread_id,
                "app-server request thread/fork timed out after 10000 ms",
            ),
        )


class DurableQueueRestoreTests(unittest.IsolatedAsyncioTestCase):
    async def test_restart_adopts_persisted_jobs_into_the_new_unique_generation(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "queue.db"
            _ = store.enqueue_queue_job(
                db_path,
                job_id="job-1",
                target_thread_id="thread-1",
                channel_id=101,
                owner_user_id=202,
                discord_message_id=303,
                app_server_generation=41,
                prompt="continue",
                queued=True,
                ack_sent=True,
            )
            snapshot = AppServerLifecycleSnapshot(99, True, 1234.0)
            deps = _Deps(db_path, lambda: snapshot, [])

            jobs = await queue_restore.restore_queue_jobs(_Bot(_Channel(101)), deps)

            self.assertEqual(len(jobs), 1)
            self.assertEqual(jobs[0].get("job_id"), "job-1")
            self.assertEqual(jobs[0].get("app_server_generation"), 99)
            records = store.list_queue_jobs(db_path)
            self.assertEqual([record.app_server_generation for record in records], [99])
            self.assertTrue(any("queue_restore_adopted" in line for line in deps.logs))

    async def test_rust_quarantine_sentinel_is_not_adopted_or_restored(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "queue.db"
            _ = store.enqueue_queue_job(
                db_path,
                job_id="quarantined",
                target_thread_id="desktop-owned",
                channel_id=404,
                owner_user_id=202,
                discord_message_id=303,
                app_server_generation=41,
                prompt="must never replay",
                queued=True,
                ack_sent=True,
            )
            _ = store.begin_queue_job_attempt(
                db_path,
                "quarantined",
                baseline_turn_ids=(),
                app_server_generation=41,
            )
            _ = store.mark_queue_job_running(
                db_path,
                "quarantined",
                "cdr-quarantined:handoff-a",
                app_server_generation=41,
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
                channel_id=101,
                owner_user_id=202,
                discord_message_id=304,
                app_server_generation=41,
                prompt="continue",
                queued=True,
                ack_sent=True,
            )
            snapshot = AppServerLifecycleSnapshot(99, True, 1234.0)
            deps = _Deps(db_path, lambda: snapshot, [])

            jobs = await queue_restore.restore_queue_jobs(_Bot(_Channel(101)), deps)

            self.assertEqual([job.get("job_id") for job in jobs], ["ordinary"])
            records = {record.job_id: record for record in store.list_queue_jobs(db_path)}
            self.assertEqual(records["quarantined"].app_server_generation, 41)
            self.assertEqual(records["ordinary"].app_server_generation, 99)
            self.assertTrue(
                any("queue_restore_quarantined skipped=1" in line for line in deps.logs)
            )

    async def test_unresolved_rust_fork_sources_are_not_adopted_or_restored(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "queue.db"
            for job_id, source in (
                ("fork-unobserved", "desktop-unobserved"),
                ("fork-staged", "desktop-staged"),
            ):
                _ = store.enqueue_queue_job(
                    db_path,
                    job_id=job_id,
                    target_thread_id=source,
                    channel_id=404,
                    owner_user_id=202,
                    discord_message_id=None,
                    app_server_generation=41,
                    prompt="must never replay",
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
                channel_id=101,
                owner_user_id=202,
                discord_message_id=304,
                app_server_generation=41,
                prompt="continue",
                queued=True,
                ack_sent=True,
            )
            snapshot = AppServerLifecycleSnapshot(99, True, 1234.0)
            deps = _Deps(db_path, lambda: snapshot, [])

            jobs = await queue_restore.restore_queue_jobs(_Bot(_Channel(101)), deps)

            self.assertEqual([job.get("job_id") for job in jobs], ["ordinary"])
            records = {record.job_id: record for record in store.list_queue_jobs(db_path)}
            self.assertEqual(records["fork-unobserved"].app_server_generation, 41)
            self.assertEqual(records["fork-staged"].app_server_generation, 41)
            self.assertEqual(records["ordinary"].app_server_generation, 99)
            self.assertTrue(
                any("queue_restore_fork_fenced skipped=2" in line for line in deps.logs)
            )

    async def test_unhealthy_start_is_retried_and_persisted_job_is_restored(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "queue.db"
            _ = store.enqueue_queue_job(
                db_path,
                job_id="job-1",
                target_thread_id="thread-1",
                channel_id=101,
                owner_user_id=202,
                discord_message_id=303,
                app_server_generation=41,
                prompt="continue",
                queued=True,
                ack_sent=True,
            )
            current = [AppServerLifecycleSnapshot(41, False, None)]
            starts: list[bool] = []

            def ensure_ready() -> None:
                starts.append(True)
                current[0] = AppServerLifecycleSnapshot(99, True, 1234.0)

            deps = _Deps(db_path, lambda: current[0], [], ensure_ready)

            jobs = await queue_restore.restore_queue_jobs(_Bot(_Channel(101)), deps)

            self.assertEqual(starts, [True])
            self.assertEqual([job.get("job_id") for job in jobs], ["job-1"])
            records = store.list_queue_jobs(db_path)
            self.assertEqual([record.app_server_generation for record in records], [99])

    async def test_generation_change_during_channel_resolution_is_readopted(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "queue.db"
            _ = store.enqueue_queue_job(
                db_path,
                job_id="job-1",
                target_thread_id="thread-1",
                channel_id=101,
                owner_user_id=202,
                discord_message_id=303,
                app_server_generation=41,
                prompt="continue",
                queued=True,
                ack_sent=True,
            )
            current = [AppServerLifecycleSnapshot(99, True, 1234.0)]

            class GenerationChangingBot(_Bot):
                def get_cached_channel_or_thread(self, channel_id: int) -> tuple[QueueJobValue, str]:
                    if current[0].generation == 99:
                        current[0] = AppServerLifecycleSnapshot(100, True, 1235.0)
                    return super().get_cached_channel_or_thread(channel_id)

            deps = _Deps(db_path, lambda: current[0], [])

            jobs = await queue_restore.restore_queue_jobs(
                GenerationChangingBot(_Channel(101)),
                deps,
            )

            self.assertEqual([job.get("job_id") for job in jobs], ["job-1"])
            self.assertEqual(jobs[0].get("app_server_generation"), 100)
            records = store.list_queue_jobs(db_path)
            self.assertEqual([record.app_server_generation for record in records], [100])
            self.assertTrue(any("queue_restore_generation_changed" in line for line in deps.logs))

    async def test_repeated_start_failure_is_surfaced_without_deleting_job(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "queue.db"
            _ = store.enqueue_queue_job(
                db_path,
                job_id="job-1",
                target_thread_id="thread-1",
                channel_id=101,
                owner_user_id=202,
                discord_message_id=303,
                app_server_generation=41,
                prompt="continue",
                queued=True,
                ack_sent=True,
            )

            def fail_start() -> None:
                raise OSError("start unavailable")

            deps = _Deps(
                db_path,
                lambda: AppServerLifecycleSnapshot(41, False, None),
                [],
                fail_start,
            )

            with self.assertRaisesRegex(queue_restore.QueueRestoreUnstableError, "start unavailable"):
                _ = await queue_restore.restore_queue_jobs(_Bot(_Channel(101)), deps)

            self.assertEqual([record.job_id for record in store.list_queue_jobs(db_path)], ["job-1"])

    async def test_repeated_healthy_generation_changes_surface_error_and_preserve_job(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "queue.db"
            _ = store.enqueue_queue_job(
                db_path,
                job_id="job-1",
                target_thread_id="thread-1",
                channel_id=101,
                owner_user_id=202,
                discord_message_id=303,
                app_server_generation=41,
                prompt="continue",
                queued=True,
                ack_sent=True,
            )
            generation = [99]

            class AlwaysChangingBot(_Bot):
                def get_cached_channel_or_thread(self, channel_id: int) -> tuple[QueueJobValue, str]:
                    generation[0] += 1
                    return super().get_cached_channel_or_thread(channel_id)

            deps = _Deps(
                db_path,
                lambda: AppServerLifecycleSnapshot(generation[0], True, 1234.0),
                [],
            )

            with self.assertRaises(queue_restore.QueueRestoreUnstableError):
                _ = await queue_restore.restore_queue_jobs(
                    AlwaysChangingBot(_Channel(101)),
                    deps,
                )

            records = store.list_queue_jobs(db_path)
            self.assertEqual([record.job_id for record in records], ["job-1"])
            self.assertEqual(records[0].app_server_generation, 101)


if __name__ == "__main__":
    _ = unittest.main()
