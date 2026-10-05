CREATE TABLE IF NOT EXISTS cdr_async_execution_obligations (
    question_id TEXT PRIMARY KEY NOT NULL,
    thread_id TEXT NOT NULL,
    origin_job_id TEXT NOT NULL,
    turn_id TEXT NOT NULL,
    channel_id INTEGER NOT NULL,
    format_version INTEGER NOT NULL,
    revision INTEGER NOT NULL,
    answer_state TEXT NOT NULL,
    execution_state TEXT NOT NULL,
    admission_state TEXT NOT NULL,
    policy TEXT NOT NULL,
    original_seal TEXT,
    claim_json TEXT NOT NULL CHECK(length(CAST(claim_json AS BLOB)) <= 131072),
    owner_json TEXT CHECK(owner_json IS NULL OR length(CAST(owner_json AS BLOB)) <= 131072),
    original_error TEXT NOT NULL,
    receipt_turn TEXT,
    terminal_proof_json TEXT,
    created_at REAL NOT NULL,
    updated_at REAL NOT NULL,
    CHECK(original_seal IS NULL OR length(CAST(original_seal AS BLOB)) <= 131072)
);
CREATE INDEX IF NOT EXISTS cdr_async_obligation_target
ON cdr_async_execution_obligations(thread_id,question_id);

CREATE TABLE IF NOT EXISTS cdr_runtime_capability_requirements (
    component TEXT PRIMARY KEY NOT NULL,
    format_version INTEGER NOT NULL CHECK(format_version > 0)
);
CREATE TRIGGER IF NOT EXISTS cdr_capability_no_downgrade
BEFORE UPDATE ON cdr_runtime_capability_requirements
WHEN NEW.component!=OLD.component OR NEW.format_version<OLD.format_version
BEGIN SELECT RAISE(ABORT,'persisted runtime capability cannot be downgraded'); END;
CREATE TRIGGER IF NOT EXISTS cdr_capability_no_delete
BEFORE DELETE ON cdr_runtime_capability_requirements
BEGIN SELECT RAISE(ABORT,'persisted runtime capability cannot be forgotten'); END;
CREATE TRIGGER IF NOT EXISTS cdr_async_require_capability
AFTER INSERT ON cdr_async_execution_obligations
BEGIN
    INSERT INTO cdr_runtime_capability_requirements(component,format_version)
    VALUES('async_resolution',NEW.format_version)
    ON CONFLICT(component) DO UPDATE SET format_version=MAX(format_version,excluded.format_version);
END;

-- A successful owned transaction certifies the exact evidence bytes. Labels or
-- any non-null JSON alone are not settlement authority. Historical settled rows
-- without this certificate remain held; no evidence is invented during upgrade.
CREATE TABLE IF NOT EXISTS cdr_async_terminal_settlements (
    question_id TEXT PRIMARY KEY NOT NULL,
    revision INTEGER NOT NULL CHECK(revision > 0),
    proof_json TEXT NOT NULL CHECK(length(CAST(proof_json AS BLOB)) <= 131072 AND json_valid(proof_json))
);
CREATE TRIGGER IF NOT EXISTS cdr_async_settlement_insert
BEFORE INSERT ON cdr_async_terminal_settlements
WHEN NOT EXISTS(SELECT 1 FROM cdr_async_execution_obligations o
    WHERE o.question_id=NEW.question_id AND o.format_version=1
    AND o.revision=NEW.revision AND o.execution_state='terminal'
    AND o.terminal_proof_json=NEW.proof_json
    AND json_extract(NEW.proof_json,'$.version')=1
    AND json_extract(NEW.proof_json,'$.owner_verified')=1
    AND json_extract(NEW.proof_json,'$.revision')=NEW.revision-1)
BEGIN SELECT RAISE(ABORT,'async settlement certificate has no matching owned decision'); END;
CREATE TRIGGER IF NOT EXISTS cdr_async_settlement_immutable
BEFORE UPDATE ON cdr_async_terminal_settlements
BEGIN SELECT RAISE(ABORT,'async settlement certificate is immutable'); END;
CREATE TRIGGER IF NOT EXISTS cdr_async_settlement_no_delete
BEFORE DELETE ON cdr_async_terminal_settlements
BEGIN SELECT RAISE(ABORT,'async settlement certificate cannot be forgotten'); END;

CREATE TABLE IF NOT EXISTS cdr_async_execution_handoffs (
    question_id TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK(revision > 0),
    evidence_json TEXT NOT NULL CHECK(length(CAST(evidence_json AS BLOB)) <= 131072 AND json_valid(evidence_json)),
    evidence_sha256 TEXT NOT NULL CHECK(length(evidence_sha256)=64),
    PRIMARY KEY(question_id,revision)
);
CREATE TRIGGER IF NOT EXISTS cdr_async_handoff_immutable
BEFORE UPDATE ON cdr_async_execution_handoffs
BEGIN SELECT RAISE(ABORT,'async execution handoff evidence is immutable'); END;
CREATE TRIGGER IF NOT EXISTS cdr_async_handoff_no_delete
BEFORE DELETE ON cdr_async_execution_handoffs
BEGIN SELECT RAISE(ABORT,'async execution handoff evidence cannot be forgotten'); END;

CREATE VIEW IF NOT EXISTS cdr_async_unsettled_obligations AS
SELECT o.* FROM cdr_async_execution_obligations o
LEFT JOIN cdr_async_terminal_settlements s ON s.question_id=o.question_id
WHERE o.format_version!=1 OR o.execution_state!='terminal' OR o.admission_state!='settled'
    OR o.policy!='ordinary' OR o.terminal_proof_json IS NULL OR s.question_id IS NULL
    OR s.revision!=o.revision OR s.proof_json!=o.terminal_proof_json;

CREATE TABLE IF NOT EXISTS cdr_async_terminal_candidates (
    question_id TEXT NOT NULL,
    revision INTEGER NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('unverified','replaced','conflict')),
    evidence_sha256 TEXT NOT NULL,
    evidence_text TEXT NOT NULL CHECK(length(CAST(evidence_text AS BLOB))<=131072),
    PRIMARY KEY(question_id,revision,kind,evidence_sha256)
);
CREATE TRIGGER IF NOT EXISTS cdr_async_candidate_immutable
BEFORE UPDATE ON cdr_async_terminal_candidates
BEGIN SELECT RAISE(ABORT,'async terminal candidate is immutable'); END;
CREATE TRIGGER IF NOT EXISTS cdr_async_candidate_no_delete
BEFORE DELETE ON cdr_async_terminal_candidates
BEGIN SELECT RAISE(ABORT,'async terminal candidate cannot be forgotten'); END;

-- Oversized legacy evidence remains in its original source, not in a truncated
-- record masquerading as a valid seal. Its source/owner cannot disappear.
CREATE TRIGGER IF NOT EXISTS cdr_async_source_no_delete
BEFORE DELETE ON cdr_async_questions
WHEN EXISTS(SELECT 1 FROM cdr_async_unsettled_obligations o WHERE o.question_id=OLD.id)
BEGIN SELECT RAISE(ABORT,'unresolved async source evidence cannot be deleted'); END;
CREATE TRIGGER IF NOT EXISTS cdr_async_source_seal_immutable
BEFORE UPDATE OF preparation_json ON cdr_async_questions
WHEN NEW.preparation_json IS NOT OLD.preparation_json
    AND EXISTS(SELECT 1 FROM cdr_async_unsettled_obligations o
        WHERE o.question_id=OLD.id AND o.original_seal IS NULL)
BEGIN SELECT RAISE(ABORT,'uncopied async source evidence must be preserved'); END;
CREATE TRIGGER IF NOT EXISTS cdr_async_uncopied_origin_guard
BEFORE DELETE ON codex_turn_queue
WHEN EXISTS(SELECT 1 FROM cdr_async_questions q
    WHERE q.origin_job_id=OLD.job_id AND q.state IN ('dispatching','submitted') AND q.dispatch_mode='steer'
    AND (q.preparation_json IS NULL OR length(CAST(q.preparation_json AS BLOB))>131072
        OR EXISTS(SELECT 1 FROM cdr_async_unsettled_obligations o
            WHERE o.question_id=q.id AND o.original_seal IS NULL)))
BEGIN SELECT RAISE(ABORT,'[cdr-rust:async-resolution-held:v1] original source evidence was not safely copied'); END;

-- Preserve the original question alongside its seal until execution is settled.
-- This only excludes protected rows from the existing body-compaction operation.
CREATE TRIGGER IF NOT EXISTS cdr_async_obligation_retention
BEFORE UPDATE OF body ON cdr_async_questions
WHEN NEW.body!=OLD.body AND EXISTS(
    SELECT 1 FROM cdr_async_unsettled_obligations o WHERE o.question_id=OLD.id)
BEGIN SELECT RAISE(IGNORE); END;

CREATE TRIGGER IF NOT EXISTS cdr_async_obligation_claim_immutable
BEFORE UPDATE OF question_id,thread_id,origin_job_id,turn_id,channel_id,
    format_version,original_seal,claim_json,owner_json,created_at
ON cdr_async_execution_obligations
BEGIN SELECT RAISE(ABORT,'async obligation original claim is immutable'); END;

CREATE TRIGGER IF NOT EXISTS cdr_async_obligation_no_forget
BEFORE DELETE ON cdr_async_execution_obligations
WHEN EXISTS(SELECT 1 FROM cdr_async_unsettled_obligations o WHERE o.question_id=OLD.question_id)
BEGIN SELECT RAISE(ABORT,'unresolved async obligation cannot be forgotten'); END;

-- The reviewed incident stays held even if bootstrap registration rolls back.
-- The policy and its capability are evidence, never a publishing approval.
CREATE TABLE IF NOT EXISTS cdr_async_recovery_policies (
    thread_id TEXT PRIMARY KEY NOT NULL,
    format_version INTEGER NOT NULL CHECK(format_version>0),
    policy TEXT NOT NULL CHECK(policy='publishing_recovery'),
    proposal_sha256 TEXT NOT NULL CHECK(length(proposal_sha256)=64),
    original_turn_id TEXT NOT NULL CHECK(length(original_turn_id) BETWEEN 1 AND 128),
    origin_job_id TEXT NOT NULL CHECK(length(origin_job_id) BETWEEN 1 AND 128),
    pending_job_id TEXT NOT NULL CHECK(length(pending_job_id) BETWEEN 1 AND 128)
);
CREATE TRIGGER IF NOT EXISTS cdr_async_recovery_policy_immutable
BEFORE UPDATE ON cdr_async_recovery_policies
BEGIN SELECT RAISE(ABORT,'reviewed recovery policy identity is immutable'); END;
CREATE TRIGGER IF NOT EXISTS cdr_async_recovery_policy_no_delete
BEFORE DELETE ON cdr_async_recovery_policies
BEGIN SELECT RAISE(ABORT,'reviewed recovery policy cannot be forgotten'); END;
CREATE TRIGGER IF NOT EXISTS cdr_async_recovery_policy_capability
AFTER INSERT ON cdr_async_recovery_policies
BEGIN
    INSERT INTO cdr_runtime_capability_requirements(component,format_version)
    VALUES('async_recovery_policy',NEW.format_version)
    ON CONFLICT(component) DO UPDATE SET format_version=MAX(format_version,excluded.format_version);
END;
-- Even a failed/absent registration needs this candidate's fallback semantics.
INSERT OR IGNORE INTO cdr_runtime_capability_requirements(component,format_version)
VALUES('async_recovery_policy',1);

CREATE TRIGGER IF NOT EXISTS cdr_async_obligation_attempt
BEFORE UPDATE OF state ON codex_turn_queue
WHEN NEW.state='starting' AND OLD.state='pending' AND (
    NEW.target_thread_id='01a06156-56cd-70b0-af02-2de7445ba4c7'
    OR EXISTS(SELECT 1 FROM cdr_async_recovery_policies p WHERE p.thread_id=NEW.target_thread_id)
    OR EXISTS(SELECT 1 FROM cdr_async_unsettled_obligations o
        WHERE o.thread_id=NEW.target_thread_id)
    OR EXISTS(SELECT 1 FROM cdr_async_questions q
        WHERE q.thread_id=NEW.target_thread_id AND q.state='dispatching' AND q.dispatch_mode='steer'
        AND NOT EXISTS(SELECT 1 FROM cdr_async_execution_obligations o WHERE o.question_id=q.id))
)
BEGIN SELECT RAISE(ABORT,'[cdr-rust:async-resolution-held:v1] original execution unresolved; no automatic retry'); END;
