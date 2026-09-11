use super::super::setup::validate_id;
use cdr_codex_state::{CodexThreadStore, ThreadResolveError, resolve_thread_ref};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use std::{collections::BTreeMap, path::Path};

#[derive(Debug, PartialEq, Eq)]
pub struct Target {
    pub channel_id: String,
    pub mirrored: bool,
}
impl Target {
    pub fn channel(id: &str) -> Result<Self, String> {
        validate_id(id)?;
        Ok(Self {
            channel_id: id.to_owned(),
            mirrored: false,
        })
    }
}
pub fn resolve(
    env: &BTreeMap<String, String>,
    root: &Path,
    reference: &str,
) -> Result<Target, String> {
    let home = ["USERPROFILE", "HOME"]
        .into_iter()
        .filter_map(|key| env.get(key))
        .map(|v| v.trim())
        .find(|v| !v.is_empty())
        .map(Path::new);
    let (_, state) =
        crate::runtime_paths::state_paths::resolve(env, home).map_err(|e| e.to_string())?;
    let (_, mirror) =
        crate::runtime_paths::store_paths::resolve(env, root, home).map_err(|e| e.to_string())?;
    resolve_databases(&state, &mirror, reference)
}
pub fn resolve_databases(state: &Path, mirror: &Path, reference: &str) -> Result<Target, String> {
    let store = CodexThreadStore::open(state).map_err(|e| e.to_string())?;
    let active = store.load_recent_threads(0).map_err(|e| e.to_string())?;
    let id = match resolve_thread_ref(&active, reference, None, false) {
        Ok(thread) => thread.id.clone(),
        Err(error @ ThreadResolveError::Ambiguous { .. }) => return Err(error.to_string()),
        Err(active_error) => {
            let archived = store.load_archived_threads(0).map_err(|e| e.to_string())?;
            resolve_thread_ref(&archived,reference,None,true)
                .map_err(|e|format!("not in active/archived threads: {reference}; active={active_error}; archived={e}"))?.id.clone()
        }
    };
    // A one-shot sender must not create, migrate or repair the running bot's DB.
    let connection = Connection::open_with_flags(mirror, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| format!("mirror mapping could not be read: {e}"))?;
    connection
        .busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|e| e.to_string())?;
    let channel=connection.query_row("SELECT discord_thread_id FROM mirror_threads WHERE codex_thread_id=?",[&id],|row|row.get::<_,i64>(0))
        .optional().map_err(|e|e.to_string())?
        .filter(|channel|*channel>0)
        .ok_or_else(||format!("stale mirror mapping: no Discord thread mapping for {id}; run !mirror check, then !mirror sync"))?;
    let owners: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM mirror_threads WHERE discord_thread_id=?",
            [channel],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    if owners != 1 {
        return Err("Discord room is mapped to multiple Codex threads; routing refused".into());
    }
    Ok(Target {
        channel_id: channel.to_string(),
        mirrored: true,
    })
}
