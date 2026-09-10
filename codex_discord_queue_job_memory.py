"""Translate durable Queue rows to the runner's mutable in-memory jobs."""

from __future__ import annotations

from codex_discord_runner_queue import QueueJob, QueueJobValue
from codex_discord_store_queue import StoredQueueJob


def to_memory_queue_job(
    record: StoredQueueJob,
    channel: QueueJobValue,
    source_message: QueueJobValue,
) -> QueueJob:
    return {
        "job_id": record.job_id,
        "channel": channel,
        "channel_id": record.channel_id,
        "owner_user_id": record.owner_user_id,
        "discord_message_id": record.discord_message_id,
        "app_server_generation": record.app_server_generation,
        "prompt": record.prompt,
        "target_thread_id": record.target_thread_id,
        "queued": record.queued,
        "ack_sent": record.ack_sent,
        "source_message": source_message,
        "state": record.state.value,
        "attempt_count": record.attempt_count,
        "turn_id": record.turn_id,
        "baseline_turn_ids": record.baseline_turn_ids,
        "last_error": record.last_error,
    }


def copy_stored_queue_state(job: QueueJob, record: StoredQueueJob) -> None:
    job["app_server_generation"] = record.app_server_generation
    job["state"] = record.state.value
    job["attempt_count"] = record.attempt_count
    job["turn_id"] = record.turn_id
    job["baseline_turn_ids"] = record.baseline_turn_ids
    job["last_error"] = record.last_error


def queue_job_int(value: int | None) -> int:
    return value or 0


def queue_job_baseline(job: QueueJob) -> set[str]:
    return set(job.get("baseline_turn_ids") or ())
