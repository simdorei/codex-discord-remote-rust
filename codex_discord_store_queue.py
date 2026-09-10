from __future__ import annotations

from dataclasses import dataclass
from enum import StrEnum, unique
import json
from pathlib import Path
import sqlite3
import time
from collections.abc import Callable
from typing import TypeAlias, cast

from codex_discord_store_connection import connect_store
from codex_discord_store_schema import init_store_schema


SQLiteCell: TypeAlias = str | int | float | bytes | None
SQLiteRow: TypeAlias = tuple[SQLiteCell, ...]
JsonScalar: TypeAlias = str | int | float | bool | None
JsonValue: TypeAlias = JsonScalar | list["JsonValue"] | dict[str, "JsonValue"]
DecodeJsonValue: TypeAlias = Callable[[str], JsonValue]
_decode_json_value: DecodeJsonValue = json.loads

QUEUE_JOB_COLUMNS = (
    "job_id, target_thread_id, channel_id, owner_user_id, discord_message_id, "
    "app_server_generation, prompt, queued, ack_sent, state, attempt_count, turn_id, "
    "baseline_turn_ids, last_error, created_at, updated_at"
)

QUARANTINED_TURN_PREFIX = "cdr-quarantined:"
QUARANTINED_ERROR_PREFIX = "[cdr-rust:app-server-fork-quarantine:v1] "
STARTING_CANDIDATE_HOLD_PREFIX = "[cdr-rust:turn-start-candidates-ambiguous:v1] "
_QUARANTINE_PREDICATE = "(state = ? AND turn_id LIKE ? AND last_error LIKE ?)"
_STARTING_HOLD_ROW_PREDICATE = "(state = ? AND turn_id IS NULL AND last_error LIKE ?)"
_STARTING_HOLD_TARGET_PREDICATE = (
    "EXISTS (SELECT 1 FROM codex_turn_queue AS starting_hold "
    "WHERE starting_hold.target_thread_id = codex_turn_queue.target_thread_id "
    "AND starting_hold.state = ? AND starting_hold.turn_id IS NULL "
    "AND starting_hold.last_error LIKE ?)"
)
_BASE_PROTECTION_PREDICATE = (
    f"({_QUARANTINE_PREDICATE} OR {_STARTING_HOLD_TARGET_PREDICATE})"
)
_FORK_HANDOFF_TABLE = "codex_thread_fork_handoffs"
_FORK_FENCE_PREDICATE = (
    "EXISTS (SELECT 1 FROM codex_thread_fork_handoffs AS unresolved_fork "
    "WHERE unresolved_fork.source_thread_id = codex_turn_queue.target_thread_id "
    "AND unresolved_fork.target_thread_id IS NULL)"
)


@unique
class QueueJobState(StrEnum):
    PENDING = "pending"
    STARTING = "starting"
    RUNNING = "running"


@dataclass(frozen=True, slots=True)
class StoredQueueJob:
    job_id: str
    target_thread_id: str
    channel_id: int
    owner_user_id: int | None
    discord_message_id: int | None
    app_server_generation: int
    prompt: str
    queued: bool
    ack_sent: bool
    state: QueueJobState
    attempt_count: int
    turn_id: str | None
    baseline_turn_ids: tuple[str, ...]
    last_error: str
    created_at: float
    updated_at: float


@dataclass(frozen=True, slots=True)
class QueueEnqueueResult:
    job: StoredQueueJob
    created: bool


@dataclass(frozen=True, slots=True)
class QueueGenerationAdoption:
    jobs: tuple[StoredQueueJob, ...]
    adopted_count: int
    quarantined_count: int
    fork_fenced_count: int
    starting_held_count: int


@dataclass(frozen=True, slots=True)
class UnresolvedForkFence:
    handoff_id: str
    source_thread_id: str
    observed_target_thread_id: str | None
    last_fork_error: str


class QueueJobNotFoundError(LookupError):
    def __init__(self, job_id: str) -> None:
        super().__init__(f"Durable queue job not found: {job_id}")
        self.job_id: str = job_id


class QueueTargetForkFencedError(RuntimeError):
    def __init__(self, fence: UnresolvedForkFence) -> None:
        observed = (
            f" observed_target={fence.observed_target_thread_id};"
            if fence.observed_target_thread_id
            else ""
        )
        fork_error = fence.last_fork_error or "no fork error was recorded"
        super().__init__(
            "Durable queue target is fenced by unresolved Rust app-server fork "
            + f"{fence.handoff_id} for {fence.source_thread_id};{observed} "
            + f"fork_error={fork_error}"
        )
        self.fence = fence


class QueueTargetStartingCandidatesHeldError(RuntimeError):
    def __init__(self, held_job: StoredQueueJob) -> None:
        super().__init__(
            "Durable queue target is held after Rust found multiple candidate turns; "
            + "manual resolution is required before another request can be queued: "
            + f"target={held_job.target_thread_id} held_job={held_job.job_id}"
        )
        self.held_job = held_job


class QueueTargetMovedError(RuntimeError):
    def __init__(
        self,
        *,
        handoff_id: str,
        source_thread_id: str,
        target_thread_id: str,
    ) -> None:
        super().__init__(
            "Durable queue target was moved by completed Rust app-server fork "
            + f"{handoff_id}: {source_thread_id} -> {target_thread_id}; "
            + "refusing stale-source enqueue"
        )
        self.handoff_id = handoff_id
        self.source_thread_id = source_thread_id
        self.target_thread_id = target_thread_id


def _quarantine_params() -> tuple[str, str, str]:
    return (
        QueueJobState.RUNNING.value,
        f"{QUARANTINED_TURN_PREFIX}%",
        f"{QUARANTINED_ERROR_PREFIX}%",
    )


def _starting_hold_params() -> tuple[str, str]:
    return (
        QueueJobState.STARTING.value,
        f"{STARTING_CANDIDATE_HOLD_PREFIX}%",
    )


def _protection_params() -> tuple[str, str, str, str, str]:
    return (*_quarantine_params(), *_starting_hold_params())


def is_quarantined_queue_values(
    state: QueueJobState | str,
    turn_id: str | None,
    last_error: str,
) -> bool:
    return (
        str(state) == QueueJobState.RUNNING.value
        and bool(turn_id)
        and str(turn_id).startswith(QUARANTINED_TURN_PREFIX)
        and last_error.startswith(QUARANTINED_ERROR_PREFIX)
    )


def is_quarantined_queue_job(job: StoredQueueJob) -> bool:
    return is_quarantined_queue_values(job.state, job.turn_id, job.last_error)


def is_starting_candidate_hold_values(
    state: QueueJobState | str,
    turn_id: str | None,
    last_error: str,
) -> bool:
    return (
        str(state) == QueueJobState.STARTING.value
        and turn_id is None
        and last_error.startswith(STARTING_CANDIDATE_HOLD_PREFIX)
    )


def is_starting_candidate_hold_job(job: StoredQueueJob) -> bool:
    return is_starting_candidate_hold_values(job.state, job.turn_id, job.last_error)


def is_protected_queue_job(job: StoredQueueJob) -> bool:
    return is_quarantined_queue_job(job) or is_starting_candidate_hold_job(job)


def _fork_handoff_columns(conn: sqlite3.Connection) -> frozenset[str] | None:
    exists = conn.execute(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?",
        (_FORK_HANDOFF_TABLE,),
    ).fetchone()
    if exists is None:
        return None
    columns = frozenset(
        str(row[1])
        for row in conn.execute(
            f"PRAGMA table_info({_FORK_HANDOFF_TABLE})"
        ).fetchall()
    )
    required = frozenset(("handoff_id", "source_thread_id", "target_thread_id"))
    missing = sorted(required - columns)
    if missing:
        raise sqlite3.DatabaseError(
            "Rust app-server fork handoff table is missing required columns: "
            + ", ".join(missing)
        )
    return columns


def _queue_protection_predicate(conn: sqlite3.Connection) -> str:
    if _fork_handoff_columns(conn) is None:
        return _BASE_PROTECTION_PREDICATE
    return f"({_BASE_PROTECTION_PREDICATE} OR {_FORK_FENCE_PREDICATE})"


def _starting_candidate_hold_for_target(
    conn: sqlite3.Connection,
    target_thread_id: str,
) -> StoredQueueJob | None:
    rows = cast(
        list[SQLiteRow],
        conn.execute(
            f"SELECT {QUEUE_JOB_COLUMNS} FROM codex_turn_queue "
            + f"WHERE target_thread_id = ? AND {_STARTING_HOLD_ROW_PREDICATE} LIMIT 2",
            (target_thread_id, *_starting_hold_params()),
        ).fetchall(),
    )
    if not rows:
        return None
    if len(rows) != 1:
        raise sqlite3.IntegrityError(
            "Multiple Rust Starting candidate holds exist for durable queue target "
            + target_thread_id
        )
    return _record(rows[0])


def _unresolved_app_server_fork_fence(
    conn: sqlite3.Connection,
    source_thread_id: str,
) -> UnresolvedForkFence | None:
    columns = _fork_handoff_columns(conn)
    if columns is None:
        return None
    observed_column = (
        "observed_target_thread_id"
        if "observed_target_thread_id" in columns
        else "NULL"
    )
    error_column = "last_fork_error" if "last_fork_error" in columns else "''"
    rows = cast(
        list[SQLiteRow],
        conn.execute(
            "SELECT handoff_id, source_thread_id, "
            + f"{observed_column}, {error_column} "
            + f"FROM {_FORK_HANDOFF_TABLE} "
            + "WHERE source_thread_id = ? AND target_thread_id IS NULL LIMIT 2",
            (source_thread_id,),
        ).fetchall(),
    )
    if not rows:
        return None
    if len(rows) != 1:
        raise sqlite3.IntegrityError(
            "Multiple unresolved Rust app-server fork handoffs fence source thread "
            + source_thread_id
        )
    row = rows[0]
    return UnresolvedForkFence(
        handoff_id=str(row[0]),
        source_thread_id=str(row[1]),
        observed_target_thread_id=str(row[2]) if row[2] is not None else None,
        last_fork_error=str(row[3] or ""),
    )


def _completed_app_server_fork_move(
    conn: sqlite3.Connection,
    source_thread_id: str,
) -> tuple[str, str, str] | None:
    if _fork_handoff_columns(conn) is None:
        return None
    rows = cast(
        list[SQLiteRow],
        conn.execute(
            "SELECT handoff_id, source_thread_id, target_thread_id "
            + f"FROM {_FORK_HANDOFF_TABLE} "
            + "WHERE source_thread_id = ? AND target_thread_id IS NOT NULL LIMIT 2",
            (source_thread_id,),
        ).fetchall(),
    )
    if not rows:
        return None
    if len(rows) != 1:
        raise sqlite3.IntegrityError(
            "Multiple completed Rust app-server fork handoffs move source thread "
            + source_thread_id
        )
    row = rows[0]
    handoff_id = str(row[0])
    source = str(row[1])
    target = str(row[2])
    if not handoff_id or not target or target != target.strip() or target == source:
        raise sqlite3.IntegrityError(
            "Invalid completed Rust app-server fork handoff for source thread "
            + source_thread_id
        )
    return handoff_id, source, target


def get_unresolved_app_server_fork_fence(
    db_path: Path,
    source_thread_id: str,
) -> UnresolvedForkFence | None:
    with _connect(db_path) as conn:
        return _unresolved_app_server_fork_fence(conn, source_thread_id)


def get_starting_candidate_hold_for_target(
    db_path: Path,
    target_thread_id: str,
) -> StoredQueueJob | None:
    with _connect(db_path) as conn:
        return _starting_candidate_hold_for_target(conn, target_thread_id)


def _connect(db_path: Path) -> sqlite3.Connection:
    conn = connect_store(db_path)
    init_store_schema(conn)
    return conn


def _record(row: SQLiteRow) -> StoredQueueJob:
    baseline = _decode_baseline(str(row[12] or "[]"))
    return StoredQueueJob(
        job_id=str(row[0]),
        target_thread_id=str(row[1]),
        channel_id=int(cast(int, row[2])),
        owner_user_id=int(cast(int, row[3])) if row[3] is not None else None,
        discord_message_id=int(cast(int, row[4])) if row[4] is not None else None,
        app_server_generation=int(cast(int, row[5])),
        prompt=str(row[6]),
        queued=bool(row[7]),
        ack_sent=bool(row[8]),
        state=QueueJobState(str(row[9])),
        attempt_count=int(cast(int, row[10])),
        turn_id=str(row[11]) if row[11] is not None else None,
        baseline_turn_ids=baseline,
        last_error=str(row[13] or ""),
        created_at=float(cast(float, row[14])),
        updated_at=float(cast(float, row[15])),
    )


def _decode_baseline(raw: str) -> tuple[str, ...]:
    decoded = _decode_json_value(raw)
    if not isinstance(decoded, list):
        return ()
    return tuple(str(value) for value in decoded)


def _select_job(conn: sqlite3.Connection, job_id: str) -> StoredQueueJob:
    row = cast(
        SQLiteRow | None,
        conn.execute(
            f"SELECT {QUEUE_JOB_COLUMNS} FROM codex_turn_queue WHERE job_id = ?",
            (job_id,),
        ).fetchone(),
    )
    if row is None:
        raise QueueJobNotFoundError(job_id)
    return _record(row)


def enqueue_queue_job(
    db_path: Path,
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
    now = time.time() if created_at is None else created_at
    with _connect(db_path) as conn:
        _ = conn.execute("BEGIN IMMEDIATE")
        held = _starting_candidate_hold_for_target(conn, target_thread_id)
        if held is not None:
            raise QueueTargetStartingCandidatesHeldError(held)
        fence = _unresolved_app_server_fork_fence(conn, target_thread_id)
        if fence is not None:
            raise QueueTargetForkFencedError(fence)
        completed_move = _completed_app_server_fork_move(conn, target_thread_id)
        if completed_move is not None:
            handoff_id, source, target = completed_move
            raise QueueTargetMovedError(
                handoff_id=handoff_id,
                source_thread_id=source,
                target_thread_id=target,
            )
        result = conn.execute(
            "INSERT OR IGNORE INTO codex_turn_queue "
            + "(job_id, target_thread_id, channel_id, owner_user_id, discord_message_id, "
            + "app_server_generation, prompt, "
            + "queued, ack_sent, state, attempt_count, baseline_turn_ids, created_at, updated_at) "
            + "VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0, '[]', ?, ?)",
            (
                job_id,
                target_thread_id,
                channel_id,
                owner_user_id,
                discord_message_id,
                app_server_generation,
                prompt,
                int(queued),
                int(ack_sent),
                QueueJobState.PENDING.value,
                now,
                now,
            ),
        )
        created = result.rowcount == 1
        if created:
            job = _select_job(conn, job_id)
        elif discord_message_id is not None:
            row = cast(
                SQLiteRow | None,
                conn.execute(
                    f"SELECT {QUEUE_JOB_COLUMNS} FROM codex_turn_queue WHERE discord_message_id = ?",
                    (discord_message_id,),
                ).fetchone(),
            )
            if row is None:
                raise QueueJobNotFoundError(job_id)
            job = _record(row)
        else:
            job = _select_job(conn, job_id)
    return QueueEnqueueResult(job, created)


def list_queue_jobs(
    db_path: Path,
    target_thread_id: str | None = None,
    *,
    app_server_generation: int | None = None,
) -> list[StoredQueueJob]:
    with _connect(db_path) as conn:
        clauses: list[str] = []
        params: list[SQLiteCell] = []
        if target_thread_id is not None:
            clauses.append("target_thread_id = ?")
            params.append(target_thread_id)
        if app_server_generation is not None:
            clauses.append("app_server_generation = ?")
            params.append(app_server_generation)
        where = f" WHERE {' AND '.join(clauses)}" if clauses else ""
        rows = cast(
            list[SQLiteRow],
            conn.execute(
                f"SELECT {QUEUE_JOB_COLUMNS} FROM codex_turn_queue{where} "
                "ORDER BY created_at, job_id",
                params,
            ).fetchall(),
        )
    return [_record(row) for row in rows]


def has_replayable_queue_jobs(db_path: Path) -> bool:
    """Return whether Python may safely replay at least one persisted queue job."""
    with _connect(db_path) as conn:
        protection = _queue_protection_predicate(conn)
        row = conn.execute(
            "SELECT EXISTS(SELECT 1 FROM codex_turn_queue "
            + f"WHERE NOT {protection})",
            _protection_params(),
        ).fetchone()
    if row is None:
        raise sqlite3.DatabaseError("Replayable durable queue guard query returned no result.")
    return bool(row[0])


def adopt_queue_jobs_generation(
    db_path: Path,
    app_server_generation: int,
) -> QueueGenerationAdoption:
    """Atomically bind persisted jobs to the current bot-owned app-server."""
    with _connect(db_path) as conn:
        _ = conn.execute("BEGIN IMMEDIATE")
        protection = _queue_protection_predicate(conn)
        quarantined = int(
            conn.execute(
                f"SELECT COUNT(*) FROM codex_turn_queue WHERE {_QUARANTINE_PREDICATE}",
                _quarantine_params(),
            ).fetchone()[0]
        )
        starting_held = int(
            conn.execute(
                f"SELECT COUNT(*) FROM codex_turn_queue WHERE {_STARTING_HOLD_ROW_PREDICATE}",
                _starting_hold_params(),
            ).fetchone()[0]
        )
        fork_fenced = 0
        if protection != _BASE_PROTECTION_PREDICATE:
            fork_fenced = int(
                conn.execute(
                    "SELECT COUNT(*) FROM codex_turn_queue "
                    + f"WHERE NOT {_BASE_PROTECTION_PREDICATE} "
                    + f"AND {_FORK_FENCE_PREDICATE}",
                    _protection_params(),
                ).fetchone()[0]
            )
        adopted = conn.execute(
            "UPDATE codex_turn_queue SET app_server_generation = ? "
            + f"WHERE app_server_generation != ? AND NOT {protection}",
            (
                app_server_generation,
                app_server_generation,
                *_protection_params(),
            ),
        ).rowcount
        rows = cast(
            list[SQLiteRow],
            conn.execute(
                f"SELECT {QUEUE_JOB_COLUMNS} FROM codex_turn_queue "
                + f"WHERE NOT {protection} ORDER BY created_at, job_id",
                _protection_params(),
            ).fetchall(),
        )
    return QueueGenerationAdoption(
        jobs=tuple(_record(row) for row in rows),
        adopted_count=max(0, adopted),
        quarantined_count=quarantined,
        fork_fenced_count=fork_fenced,
        starting_held_count=starting_held,
    )


def discard_queue_jobs_for_generation(
    db_path: Path,
    app_server_generation: int | None,
) -> list[StoredQueueJob]:
    """Atomically remove jobs that cannot run in the supplied server generation.

    ``None`` means that the app server is unhealthy, so every queued job is stale.
    """
    with _connect(db_path) as conn:
        _ = conn.execute("BEGIN IMMEDIATE")
        protection = _queue_protection_predicate(conn)
        if app_server_generation is None:
            where = f" WHERE NOT {protection}"
            params: tuple[SQLiteCell, ...] = _protection_params()
        else:
            where = (
                " WHERE app_server_generation != ? "
                + f"AND NOT {protection}"
            )
            params = (app_server_generation, *_protection_params())
        rows = cast(
            list[SQLiteRow],
            conn.execute(
                f"SELECT {QUEUE_JOB_COLUMNS} FROM codex_turn_queue{where} "
                "ORDER BY created_at, job_id",
                params,
            ).fetchall(),
        )
        _ = conn.execute(f"DELETE FROM codex_turn_queue{where}", params)
    return [_record(row) for row in rows]


def discard_observed_queue_jobs(
    db_path: Path,
    observed_jobs: list[StoredQueueJob] | tuple[StoredQueueJob, ...],
) -> list[StoredQueueJob]:
    """Atomically remove rows only while their observed generation still matches."""
    ids_by_generation: dict[int, dict[str, None]] = {}
    for job in observed_jobs:
        if is_protected_queue_job(job):
            continue
        ids_by_generation.setdefault(job.app_server_generation, {})[job.job_id] = None
    if not ids_by_generation:
        return []
    rows: list[SQLiteRow] = []
    with _connect(db_path) as conn:
        _ = conn.execute("BEGIN IMMEDIATE")
        protection = _queue_protection_predicate(conn)
        for generation, generation_ids in ids_by_generation.items():
            unique_ids = tuple(generation_ids)
            for offset in range(0, len(unique_ids), 499):
                chunk = unique_ids[offset : offset + 499]
                placeholders = ",".join("?" for _ in chunk)
                params: tuple[SQLiteCell, ...] = (
                    *chunk,
                    generation,
                    *_protection_params(),
                )
                where = (
                    f"job_id IN ({placeholders}) AND app_server_generation = ? "
                    + f"AND NOT {protection}"
                )
                rows.extend(
                    cast(
                        list[SQLiteRow],
                        conn.execute(
                            f"SELECT {QUEUE_JOB_COLUMNS} FROM codex_turn_queue WHERE {where}",
                            params,
                        ).fetchall(),
                    )
                )
                _ = conn.execute(
                    f"DELETE FROM codex_turn_queue WHERE {where}",
                    params,
                )
    records = [_record(row) for row in rows]
    return sorted(records, key=lambda record: (record.created_at, record.job_id))


def begin_queue_job_attempt(
    db_path: Path,
    job_id: str,
    *,
    baseline_turn_ids: tuple[str, ...],
    app_server_generation: int,
) -> StoredQueueJob:
    with _connect(db_path) as conn:
        now = time.time()
        protection = _queue_protection_predicate(conn)
        result = conn.execute(
            "UPDATE codex_turn_queue SET state = ?, attempt_count = attempt_count + 1, "
            + "turn_id = NULL, baseline_turn_ids = ?, last_error = '', updated_at = ? "
            + "WHERE job_id = ? AND app_server_generation = ? "
            + f"AND NOT {protection}",
            (
                QueueJobState.STARTING.value,
                json.dumps(baseline_turn_ids),
                now,
                job_id,
                app_server_generation,
                *_protection_params(),
            ),
        )
        if result.rowcount != 1:
            raise QueueJobNotFoundError(job_id)
        return _select_job(conn, job_id)


def mark_queue_job_running(
    db_path: Path,
    job_id: str,
    turn_id: str,
    *,
    app_server_generation: int,
) -> StoredQueueJob:
    with _connect(db_path) as conn:
        protection = _queue_protection_predicate(conn)
        result = conn.execute(
            "UPDATE codex_turn_queue SET state = ?, turn_id = ?, updated_at = ? "
            + "WHERE job_id = ? AND app_server_generation = ? "
            + f"AND NOT {protection}",
            (
                QueueJobState.RUNNING.value,
                turn_id,
                time.time(),
                job_id,
                app_server_generation,
                *_protection_params(),
            ),
        )
        if result.rowcount != 1:
            raise QueueJobNotFoundError(job_id)
        return _select_job(conn, job_id)


def complete_queue_job(db_path: Path, job_id: str) -> bool:
    with _connect(db_path) as conn:
        protection = _queue_protection_predicate(conn)
        result = conn.execute(
            "DELETE FROM codex_turn_queue WHERE job_id = ? "
            + f"AND NOT {protection}",
            (job_id, *_protection_params()),
        )
        return result.rowcount == 1


def flush_queue_jobs(
    db_path: Path,
    target_thread_id: str,
    *,
    app_server_generation: int,
) -> list[StoredQueueJob]:
    with _connect(db_path) as conn:
        _ = conn.execute("BEGIN IMMEDIATE")
        protection = _queue_protection_predicate(conn)
        rows = cast(
            list[SQLiteRow],
            conn.execute(
                f"SELECT {QUEUE_JOB_COLUMNS} FROM codex_turn_queue "
                + "WHERE target_thread_id = ? AND app_server_generation = ? "
                + f"AND NOT {protection} ORDER BY created_at, job_id",
                (target_thread_id, app_server_generation, *_protection_params()),
            ).fetchall(),
        )
        _ = conn.execute(
            "DELETE FROM codex_turn_queue "
            + "WHERE target_thread_id = ? AND app_server_generation = ? "
            + f"AND NOT {protection}",
            (target_thread_id, app_server_generation, *_protection_params()),
        )
    return [_record(row) for row in rows]


def retract_queue_job(
    db_path: Path,
    target_thread_id: str,
    *,
    channel_id: int | None,
    owner_user_id: int | None,
) -> StoredQueueJob | None:
    clauses = ["target_thread_id = ?", "state = ?"]
    params: list[SQLiteCell] = [target_thread_id, QueueJobState.PENDING.value]
    if channel_id is not None:
        clauses.append("channel_id = ?")
        params.append(channel_id)
    if owner_user_id is not None:
        clauses.append("owner_user_id = ?")
        params.append(owner_user_id)
    with _connect(db_path) as conn:
        _ = conn.execute("BEGIN IMMEDIATE")
        protection = _queue_protection_predicate(conn)
        clauses.append(f"NOT {protection}")
        params.extend(_protection_params())
        where = " AND ".join(clauses)
        row = cast(
            SQLiteRow | None,
            conn.execute(
                f"SELECT {QUEUE_JOB_COLUMNS} FROM codex_turn_queue WHERE {where} "
                "ORDER BY created_at DESC, job_id DESC LIMIT 1",
                params,
            ).fetchone(),
        )
        if row is None:
            return None
        job = _record(row)
        _ = conn.execute("DELETE FROM codex_turn_queue WHERE job_id = ?", (job.job_id,))
        return job
