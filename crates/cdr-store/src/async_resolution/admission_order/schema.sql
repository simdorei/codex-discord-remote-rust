CREATE TABLE IF NOT EXISTS cdr_recovery_ingress_order (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT CHECK(sequence>0),
    ingress_id TEXT UNIQUE,
    kind TEXT NOT NULL CHECK(kind IN ('message','interaction','action')),
    event_id INTEGER,
    origin TEXT NOT NULL CHECK(origin IN ('legacy','admitted')),
    identity_sha256 TEXT,
    UNIQUE(kind,event_id),
    CHECK((origin='legacy' AND identity_sha256 IS NULL)
        OR (origin='admitted' AND ingress_id IS NOT NULL AND identity_sha256 IS NOT NULL
            AND length(identity_sha256)=64 AND identity_sha256 NOT GLOB '*[^0-9a-f]*'))
);
-- object --
CREATE TRIGGER IF NOT EXISTS cdr_recovery_ingress_order_no_update
BEFORE UPDATE ON cdr_recovery_ingress_order
BEGIN SELECT RAISE(ABORT,'first admission order is immutable'); END;
-- object --
CREATE TRIGGER IF NOT EXISTS cdr_recovery_ingress_order_no_delete
BEFORE DELETE ON cdr_recovery_ingress_order
BEGIN SELECT RAISE(ABORT,'first admission order cannot be forgotten'); END;
-- object --
CREATE TRIGGER IF NOT EXISTS cdr_recovery_ingress_order_no_replace
BEFORE INSERT ON cdr_recovery_ingress_order
WHEN EXISTS(SELECT 1 FROM cdr_recovery_ingress_order o WHERE o.sequence=NEW.sequence
    OR (NEW.ingress_id IS NOT NULL AND o.ingress_id=NEW.ingress_id)
    OR (NEW.event_id IS NOT NULL AND o.kind=NEW.kind AND o.event_id=NEW.event_id))
BEGIN SELECT RAISE(ABORT,'an old ingress cannot acquire a new admission order'); END;
