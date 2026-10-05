use crate::Result;
use rusqlite::Connection;

pub(crate) fn migrate_schema(db: &Connection) -> Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS cdr_mirror_container_creations (
         kind TEXT NOT NULL CHECK(kind IN ('category','project')),scope_key TEXT NOT NULL CHECK(length(scope_key)>0),
         token TEXT NOT NULL UNIQUE,guild_id INTEGER NOT NULL CHECK(guild_id>0),
         parent_id INTEGER,expected_json TEXT NOT NULL,phase TEXT NOT NULL,channel_id INTEGER,
         PRIMARY KEY(kind,scope_key),
         CHECK((kind='category' AND parent_id IS NULL) OR (kind='project' AND parent_id IS NOT NULL AND parent_id>0)),
         CHECK((phase='attempted' AND channel_id IS NULL)
           OR (phase='confirmed' AND channel_id IS NOT NULL AND channel_id>0)
           OR (phase='bound' AND kind='category' AND channel_id IS NOT NULL AND channel_id>0)));",
    )?;
    Ok(())
}

pub(crate) fn schema_current(db: &Connection) -> Result<bool> {
    Ok(db.query_row(
        "SELECT COUNT(*)=8 FROM pragma_table_info('cdr_mirror_container_creations')
         WHERE name IN ('kind','scope_key','token','guild_id','parent_id','expected_json','phase','channel_id')",
        [], |r| r.get(0),
    )?)
}
