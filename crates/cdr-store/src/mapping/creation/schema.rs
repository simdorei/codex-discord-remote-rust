use crate::Result;
use rusqlite::Connection;

pub(crate) fn migrate_schema(db: &Connection) -> Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS cdr_mirror_thread_creations (
         thread_id TEXT PRIMARY KEY NOT NULL,token TEXT NOT NULL UNIQUE,
         guild_id INTEGER NOT NULL CHECK(guild_id>0),parent_id INTEGER NOT NULL CHECK(parent_id>0),
         expected_parent_id INTEGER,expected_channel_id INTEGER,
         phase TEXT NOT NULL CHECK(phase IN ('attempted','confirmed')),channel_id INTEGER,
         CHECK((expected_parent_id IS NULL AND expected_channel_id IS NULL)
            OR (expected_parent_id IS NOT NULL AND expected_channel_id IS NOT NULL
                AND expected_parent_id>0 AND expected_channel_id>0)),
         CHECK((phase='attempted' AND channel_id IS NULL)
            OR (phase='confirmed' AND channel_id IS NOT NULL AND channel_id>0)));",
    )?;
    Ok(())
}

pub(crate) fn schema_current(db: &Connection) -> Result<bool> {
    Ok(db.query_row(
        "SELECT COUNT(*)=8 FROM pragma_table_info('cdr_mirror_thread_creations')
         WHERE name IN ('thread_id','token','guild_id','parent_id','expected_parent_id',
                       'expected_channel_id','phase','channel_id')",
        [],
        |row| row.get(0),
    )?)
}
