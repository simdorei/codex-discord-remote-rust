"""Read-only, exact-job preview. Never grants delivery/replay authority."""
from __future__ import annotations

import argparse
from contextlib import closing
import hashlib
import json
from pathlib import Path
import sqlite3


def inspect(database: Path, job: str) -> dict[str, object]:
    with closing(sqlite3.connect(database.resolve().as_uri() + "?mode=ro", uri=True)) as db:
        db.row_factory = sqlite3.Row
        db.execute("PRAGMA query_only=ON")
        db.execute("BEGIN")
        rows = db.execute(
            "SELECT ingress_id,kind,event_id,channel_id,target_thread_id,outcome_json,"
            "confirmation_delivered FROM discord_ingress_journal WHERE owner_id=?", (job,)
        ).fetchall()
        outbox = db.execute(
            "SELECT target_thread_id,turn_id,channel_id,content FROM codex_delivery_outbox WHERE job_id=?", (job,)
        ).fetchall()
        report: dict[str, object] = {
            "read_only": True, "job_id": job, "ingress_count": len(rows),
            "outbox_count": len(outbox), "replay_authorized": False,
            "delivery_authorized": False,
        }
        if len(rows) != 1 or len(outbox) != 1:
            return {**report, "reason": "exact ingress/outbox identity is not unique; preserved"}
        owner, final = rows[0], outbox[0]
        evidence = json.loads(owner["outcome_json"] or "{}")
        prepared = evidence.get("new_verification", {})
        mapping = db.execute(
            "SELECT codex_thread_id FROM mirror_threads WHERE discord_thread_id=?", (final["channel_id"],)
        ).fetchall()
        identity_matches = (
            owner["target_thread_id"] == final["target_thread_id"]
            and prepared.get("thread_id") == final["target_thread_id"]
            and prepared.get("channel_id") == final["channel_id"]
            and len(mapping) == 1 and mapping[0][0] == final["target_thread_id"]
        )
        key = json.dumps([owner["channel_id"], "message/reply/v1",
            f'inbound-message/{owner["event_id"]}/action-result', 0], separators=(",", ":"))
        receipt = db.execute(
            "SELECT message_id,retryable,blocked_reason FROM codex_delivery_receipts WHERE receipt_key=?", (key,)
        ).fetchone()
        receipt_state = "absent" if receipt is None else (
            "confirmed" if receipt["message_id"] else
            "rejected_blocked" if receipt["blocked_reason"] else
            "definite_rejection" if receipt["retryable"] else "unknown"
        )
        report.update({"thread_id": final["target_thread_id"], "turn_id": final["turn_id"],
            "destination_matches": identity_matches, "normal_ack_receipt": receipt_state,
            "final_sha256": hashlib.sha256(final["content"].encode("utf-8")).hexdigest(),
            "normal_ack_confirmed_flag": bool(owner["confirmation_delivered"]),
            "reason": "Historical non-attempt and current exclusive delivery ownership are not proven by this preview; preserved"})
        return report


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--database", required=True, type=Path)
    parser.add_argument("--job-id", required=True)
    args = parser.parse_args()
    print(json.dumps(inspect(args.database, args.job_id), ensure_ascii=False))


if __name__ == "__main__":
    main()
