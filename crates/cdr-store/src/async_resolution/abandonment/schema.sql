CREATE TABLE IF NOT EXISTS cdr_recovery_abandonment_proposals (
    id TEXT PRIMARY KEY NOT NULL CHECK(length(id)=32 AND id NOT GLOB '*[^0-9a-f]*'),
    format_version INTEGER NOT NULL CHECK(format_version=1),
    revision INTEGER NOT NULL CHECK(revision>0),
    job_id TEXT NOT NULL,
    target_thread_id TEXT NOT NULL,
    owner_user_id INTEGER NOT NULL CHECK(owner_user_id>0),
    channel_id INTEGER NOT NULL CHECK(channel_id>0),
    application_id INTEGER NOT NULL CHECK(application_id>0),
    seal_json TEXT NOT NULL CHECK(json_valid(seal_json) AND length(CAST(seal_json AS BLOB))<=524288),
    seal_sha256 TEXT NOT NULL CHECK(length(seal_sha256)=64 AND seal_sha256 NOT GLOB '*[^0-9a-f]*'),
    UNIQUE(job_id,revision)
);

-- object --

CREATE TABLE IF NOT EXISTS cdr_recovery_abandonment_deliveries (
    proposal_id TEXT PRIMARY KEY NOT NULL,
    revision INTEGER NOT NULL CHECK(revision>0),
    message_id INTEGER NOT NULL UNIQUE CHECK(message_id>0),
    body_sha256 TEXT NOT NULL CHECK(length(body_sha256)=64 AND body_sha256 NOT GLOB '*[^0-9a-f]*')
);

-- object --

CREATE TABLE IF NOT EXISTS cdr_recovery_abandonment_decisions (
    proposal_id TEXT PRIMARY KEY NOT NULL,
    revision INTEGER NOT NULL CHECK(revision>0),
    ingress_id TEXT NOT NULL UNIQUE,
    interaction_id INTEGER NOT NULL UNIQUE CHECK(interaction_id>0),
    decision TEXT NOT NULL CHECK(decision IN ('abandon_only','keep_held')),
    recorded_at_bits TEXT NOT NULL
);

-- object --

CREATE TRIGGER IF NOT EXISTS cdr_recovery_abandonment_proposal_immutable
BEFORE UPDATE ON cdr_recovery_abandonment_proposals
BEGIN SELECT RAISE(ABORT,'abandonment evidence is immutable'); END;

-- object --

CREATE TRIGGER IF NOT EXISTS cdr_recovery_abandonment_proposal_no_delete
BEFORE DELETE ON cdr_recovery_abandonment_proposals
BEGIN SELECT RAISE(ABORT,'abandonment evidence cannot be forgotten'); END;

-- object --

CREATE TRIGGER IF NOT EXISTS cdr_recovery_abandonment_proposal_no_replace
BEFORE INSERT ON cdr_recovery_abandonment_proposals
WHEN EXISTS(SELECT 1 FROM cdr_recovery_abandonment_proposals WHERE id=NEW.id OR (job_id=NEW.job_id AND revision=NEW.revision))
BEGIN SELECT RAISE(ABORT,'abandonment evidence cannot be replaced'); END;

-- object --

CREATE TRIGGER IF NOT EXISTS cdr_recovery_abandonment_delivery_immutable
BEFORE UPDATE ON cdr_recovery_abandonment_deliveries
BEGIN SELECT RAISE(ABORT,'abandonment evidence is immutable'); END;

-- object --

CREATE TRIGGER IF NOT EXISTS cdr_recovery_abandonment_delivery_no_delete
BEFORE DELETE ON cdr_recovery_abandonment_deliveries
BEGIN SELECT RAISE(ABORT,'abandonment evidence cannot be forgotten'); END;

-- object --

CREATE TRIGGER IF NOT EXISTS cdr_recovery_abandonment_delivery_no_replace
BEFORE INSERT ON cdr_recovery_abandonment_deliveries
WHEN EXISTS(SELECT 1 FROM cdr_recovery_abandonment_deliveries WHERE proposal_id=NEW.proposal_id OR message_id=NEW.message_id)
BEGIN SELECT RAISE(ABORT,'abandonment evidence cannot be replaced'); END;

-- object --

CREATE TRIGGER IF NOT EXISTS cdr_recovery_abandonment_decision_immutable
BEFORE UPDATE ON cdr_recovery_abandonment_decisions
BEGIN SELECT RAISE(ABORT,'abandonment evidence is immutable'); END;

-- object --

CREATE TRIGGER IF NOT EXISTS cdr_recovery_abandonment_decision_no_delete
BEFORE DELETE ON cdr_recovery_abandonment_decisions
BEGIN SELECT RAISE(ABORT,'abandonment evidence cannot be forgotten'); END;

-- object --

CREATE TRIGGER IF NOT EXISTS cdr_recovery_abandonment_decision_no_replace
BEFORE INSERT ON cdr_recovery_abandonment_decisions
WHEN EXISTS(SELECT 1 FROM cdr_recovery_abandonment_decisions WHERE proposal_id=NEW.proposal_id OR ingress_id=NEW.ingress_id OR interaction_id=NEW.interaction_id)
BEGIN SELECT RAISE(ABORT,'abandonment evidence cannot be replaced'); END;

-- object --

CREATE TRIGGER IF NOT EXISTS cdr_recovery_abandonment_cancellation_no_update
BEFORE UPDATE ON codex_request_cancellations
WHEN EXISTS(SELECT 1 FROM cdr_recovery_abandonment_proposals p
    JOIN cdr_recovery_abandonment_decisions d ON d.proposal_id=p.id AND d.revision=p.revision
    WHERE p.job_id=OLD.job_id AND d.decision='abandon_only')
BEGIN SELECT RAISE(ABORT,'abandoned request must remain non-replayable'); END;

-- object --

CREATE TRIGGER IF NOT EXISTS cdr_recovery_abandonment_cancellation_no_delete
BEFORE DELETE ON codex_request_cancellations
WHEN EXISTS(SELECT 1 FROM cdr_recovery_abandonment_proposals p
    JOIN cdr_recovery_abandonment_decisions d ON d.proposal_id=p.id AND d.revision=p.revision
    WHERE p.job_id=OLD.job_id AND d.decision='abandon_only')
BEGIN SELECT RAISE(ABORT,'abandoned request must remain non-replayable'); END;

-- object --

CREATE TRIGGER IF NOT EXISTS cdr_recovery_abandonment_cancellation_no_replace
BEFORE INSERT ON codex_request_cancellations
WHEN EXISTS(SELECT 1 FROM codex_request_cancellations c
    JOIN cdr_recovery_abandonment_proposals p ON p.job_id=c.job_id
    JOIN cdr_recovery_abandonment_decisions d ON d.proposal_id=p.id AND d.revision=p.revision
    WHERE d.decision='abandon_only' AND (c.job_id=NEW.job_id
        OR (NEW.discord_message_id IS NOT NULL AND c.discord_message_id=NEW.discord_message_id)))
BEGIN SELECT RAISE(ABORT,'abandoned cancellation cannot be replaced'); END;
