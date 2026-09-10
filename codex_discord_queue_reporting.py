from __future__ import annotations

from codex_discord_queue_processor import QueueJobSummary


def build_queue_retry_message(reason: str) -> str:
    return "Queued Codex turn did not complete. Retrying this request once.\n" + f"Reason: {reason}"


def build_queue_batch_failure_message(
    reason: str,
    deleted_jobs: list[QueueJobSummary],
) -> str:
    previews = [f"- {item.prompt.strip()[:120] or '(empty request)'}" for item in deleted_jobs[:10]]
    if len(deleted_jobs) > 10:
        previews.append(f"- ... and {len(deleted_jobs) - 10} more")
    return "\n".join(
        (
            "Queue batch stopped because the recovery turn also failed.",
            f"Reason: {reason}",
            f"Deleted queued requests: {len(deleted_jobs)}",
            *previews,
        )
    )
