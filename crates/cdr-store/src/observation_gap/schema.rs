use super::{Connection, Result};
pub(crate) fn migrate_schema(db: &Connection) -> Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS cdr_observation_streams (
        owner_id TEXT NOT NULL,generation INTEGER NOT NULL,active INTEGER NOT NULL,
        seen_seq INTEGER NOT NULL DEFAULT 0,scan_after INTEGER NOT NULL DEFAULT 0,
        cycle_upper INTEGER NOT NULL DEFAULT 0,unsealed INTEGER NOT NULL DEFAULT 1,
        PRIMARY KEY(owner_id,generation));
        CREATE TABLE IF NOT EXISTS cdr_observation_gaps (
        gap_id INTEGER PRIMARY KEY AUTOINCREMENT,owner_id TEXT NOT NULL,generation INTEGER NOT NULL,
        first_seq INTEGER NOT NULL,last_seq INTEGER NOT NULL,scan_cursor INTEGER NOT NULL,
        revision INTEGER NOT NULL DEFAULT 0,state TEXT NOT NULL CHECK(state IN ('Open','Unresolved','Verified')),
        verified_json TEXT NOT NULL DEFAULT '[]',detail TEXT NOT NULL DEFAULT '',
        CHECK(first_seq>=0 AND last_seq>=first_seq AND scan_cursor>=first_seq-1 AND scan_cursor<=last_seq));
        CREATE INDEX IF NOT EXISTS cdr_observation_gap_scope ON cdr_observation_gaps(owner_id,generation,gap_id);
        CREATE UNIQUE INDEX IF NOT EXISTS cdr_observation_gap_unknown ON cdr_observation_gaps(owner_id,generation) WHERE first_seq=0;")?;
    Ok(())
}
pub(crate) fn schema_current(db: &Connection) -> Result<bool> {
    Ok(db.query_row("SELECT COUNT(*)=2 FROM sqlite_schema WHERE type='table'
        AND name IN ('cdr_observation_streams','cdr_observation_gaps')",[],|r|r.get::<_,bool>(0))?
        && db.query_row("SELECT COUNT(*)=9 FROM pragma_table_info('cdr_observation_gaps')
        WHERE name IN ('gap_id','owner_id','generation','first_seq','last_seq','scan_cursor','revision','state','verified_json')",[],|r|r.get::<_,bool>(0))?)
}
