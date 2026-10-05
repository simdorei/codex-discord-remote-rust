CREATE TABLE IF NOT EXISTS cdr_recovery_publication_proposals (
    id TEXT PRIMARY KEY NOT NULL CHECK(length(id)=32),
    format_version INTEGER NOT NULL CHECK(format_version=1),
    revision INTEGER NOT NULL CHECK(revision>0),
    job_id TEXT NOT NULL,
    target_thread_id TEXT NOT NULL,
    owner_user_id INTEGER NOT NULL CHECK(owner_user_id>0),
    channel_id INTEGER NOT NULL CHECK(channel_id>0),
    application_id INTEGER NOT NULL CHECK(application_id>0),
    seal_json TEXT NOT NULL CHECK(json_valid(seal_json) AND length(CAST(seal_json AS BLOB))<=524288),
    seal_sha256 TEXT NOT NULL CHECK(length(seal_sha256)=64)
);
CREATE UNIQUE INDEX IF NOT EXISTS cdr_recovery_publication_job_revision
ON cdr_recovery_publication_proposals(job_id,revision);
CREATE TABLE IF NOT EXISTS cdr_recovery_publication_deliveries (
    proposal_id TEXT PRIMARY KEY NOT NULL,
    revision INTEGER NOT NULL CHECK(revision>0),
    message_id INTEGER NOT NULL CHECK(message_id>0),
    body_sha256 TEXT NOT NULL CHECK(length(body_sha256)=64)
);
CREATE TABLE IF NOT EXISTS cdr_recovery_publication_decisions (
    proposal_id TEXT PRIMARY KEY NOT NULL,
    revision INTEGER NOT NULL CHECK(revision>0),
    ingress_id TEXT NOT NULL UNIQUE,
    interaction_id INTEGER NOT NULL UNIQUE CHECK(interaction_id>0),
    decision TEXT NOT NULL CHECK(decision IN ('approve_exact','keep_held')),
    recorded_at_bits TEXT NOT NULL
);
CREATE TRIGGER IF NOT EXISTS cdr_recovery_publication_proposal_immutable
BEFORE UPDATE ON cdr_recovery_publication_proposals
BEGIN SELECT RAISE(ABORT,'publication proposal is immutable'); END;
CREATE TRIGGER IF NOT EXISTS cdr_recovery_publication_proposal_no_delete
BEFORE DELETE ON cdr_recovery_publication_proposals
BEGIN SELECT RAISE(ABORT,'publication proposal cannot be forgotten'); END;
CREATE TRIGGER IF NOT EXISTS cdr_recovery_publication_delivery_immutable
BEFORE UPDATE ON cdr_recovery_publication_deliveries
BEGIN SELECT RAISE(ABORT,'publication delivery binding is immutable'); END;
CREATE TRIGGER IF NOT EXISTS cdr_recovery_publication_delivery_no_delete
BEFORE DELETE ON cdr_recovery_publication_deliveries
BEGIN SELECT RAISE(ABORT,'publication delivery cannot be forgotten'); END;
CREATE TRIGGER IF NOT EXISTS cdr_recovery_publication_decision_immutable
BEFORE UPDATE ON cdr_recovery_publication_decisions
BEGIN SELECT RAISE(ABORT,'publication decision is immutable'); END;
CREATE TRIGGER IF NOT EXISTS cdr_recovery_publication_decision_no_delete
BEFORE DELETE ON cdr_recovery_publication_decisions
BEGIN SELECT RAISE(ABORT,'publication decision cannot be forgotten'); END;
