CREATE VIEW IF NOT EXISTS cdr_archive_inspections_v1 AS
SELECT ingress_id FROM discord_ingress_journal WHERE
    (kind='message' AND json_extract(payload_json,'$.version')=1 AND (
        json_extract(payload_json,'$.plan.Execute') IN ('Help','Runners','Doctor','Where','Identity','Resources')
        OR json_type(payload_json,'$.plan.Execute.SavedRequest')='object'))
    OR (kind='interaction' AND json_extract(payload_json,'$.version')=1
        AND json_extract(payload_json,'$.work.Slash.name') IN ('help','runners','doctor','where'));
