CREATE TABLE IF NOT EXISTS codex_archive_fences (
    target_thread_id TEXT PRIMARY KEY,
    operation_id TEXT NOT NULL,
    own_ingress_id TEXT,
    phase TEXT NOT NULL CHECK(phase IN ('attempted','verified'))
);

-- Keep the help and saved-request inspection path usable while work is held.
-- Only already-frozen, explicitly read-only commands are exempt, never prompts.
CREATE VIEW IF NOT EXISTS cdr_archive_inspections_v1 AS
SELECT ingress_id FROM discord_ingress_journal WHERE
    (kind='message' AND json_extract(payload_json,'$.version')=1 AND (
        json_extract(payload_json,'$.plan.Execute') IN ('Help','Runners','Doctor','Where','Identity','Resources')
        OR json_type(payload_json,'$.plan.Execute.SavedRequest')='object'))
    OR (kind='interaction' AND json_extract(payload_json,'$.version')=1
        AND json_extract(payload_json,'$.work.Slash.name') IN ('help','runners','doctor','where'));

CREATE TRIGGER IF NOT EXISTS cdr_archive_admission_v1
AFTER INSERT ON discord_ingress_journal
WHEN NOT EXISTS(SELECT 1 FROM cdr_archive_inspections_v1 WHERE ingress_id=NEW.ingress_id)
AND EXISTS(SELECT 1 FROM codex_archive_fences f
    WHERE f.target_thread_id=NEW.target_thread_id
       OR (NEW.target_thread_id IS NULL AND f.phase='attempted'))
BEGIN
    UPDATE discord_ingress_journal SET state='held',phase='archive_fenced',
        hold_reason='archive scope is reserved or archived; request is saved, not executed and will not be retried automatically'
    WHERE ingress_id=NEW.ingress_id;
END;

CREATE TRIGGER IF NOT EXISTS cdr_archive_execution_v1
BEFORE UPDATE OF state,target_thread_id ON discord_ingress_journal
WHEN NEW.state IN ('executing','owned')
AND NOT EXISTS(SELECT 1 FROM cdr_archive_inspections_v1 WHERE ingress_id=NEW.ingress_id)
AND EXISTS(
    SELECT 1 FROM codex_archive_fences f WHERE
    (f.target_thread_id=NEW.target_thread_id OR (NEW.target_thread_id IS NULL AND f.phase='attempted'))
    AND (f.own_ingress_id IS NULL OR f.own_ingress_id!=NEW.ingress_id))
BEGIN SELECT RAISE(ABORT,'archive fence prevents execution or ownership handoff'); END;

CREATE TRIGGER IF NOT EXISTS cdr_archive_held_v1
BEFORE UPDATE OF state ON discord_ingress_journal
WHEN OLD.state='held' AND OLD.phase='archive_fenced' AND NEW.state!='held'
BEGIN SELECT RAISE(ABORT,'archive-held request requires explicit review; no automatic replay'); END;

CREATE TRIGGER IF NOT EXISTS cdr_archive_queue_insert_v1
BEFORE INSERT ON codex_turn_queue
WHEN EXISTS(SELECT 1 FROM codex_archive_fences WHERE target_thread_id=NEW.target_thread_id)
BEGIN SELECT RAISE(ABORT,'archive fence prevents queue handoff'); END;

CREATE TRIGGER IF NOT EXISTS cdr_archive_queue_update_v1
BEFORE UPDATE OF target_thread_id,state ON codex_turn_queue
WHEN EXISTS(SELECT 1 FROM codex_archive_fences WHERE target_thread_id=NEW.target_thread_id)
BEGIN SELECT RAISE(ABORT,'archive fence prevents queue execution or retargeting'); END;

CREATE TRIGGER IF NOT EXISTS cdr_archive_intake_insert_v1
BEFORE INSERT ON codex_prompt_intakes
WHEN EXISTS(SELECT 1 FROM codex_archive_fences WHERE target_thread_id=NEW.target_thread_id)
BEGIN SELECT RAISE(ABORT,'archive fence prevents prompt intake handoff'); END;

CREATE TRIGGER IF NOT EXISTS cdr_archive_intake_update_v1
BEFORE UPDATE OF target_thread_id,claim_token ON codex_prompt_intakes
WHEN EXISTS(SELECT 1 FROM codex_archive_fences WHERE target_thread_id=NEW.target_thread_id)
BEGIN SELECT RAISE(ABORT,'archive fence prevents prompt intake execution or retargeting'); END;
