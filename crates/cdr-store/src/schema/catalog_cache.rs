//! Cache structural validation only, never data, ownership, or execution guards.
use std::collections::VecDeque;
use std::sync::Mutex;

use rusqlite::Connection;

use crate::Result;

pub(super) type Signature = Box<str>;
const MAX_CATALOGS: usize = 64;
const MAX_CATALOG_BYTES: usize = 4 * 1024 * 1024;

#[derive(Default)]
struct CatalogCache {
    entries: VecDeque<Signature>,
    bytes: usize,
}

impl CatalogCache {
    fn remember(&mut self, signature: Signature) {
        let size = signature.len();
        if size > MAX_CATALOG_BYTES || self.entries.contains(&signature) {
            return;
        }
        while self.entries.len() >= MAX_CATALOGS || self.bytes + size > MAX_CATALOG_BYTES {
            let Some(old) = self.entries.pop_front() else {
                return;
            };
            self.bytes -= old.len();
        }
        self.bytes += size;
        self.entries.push_back(signature);
    }
}

static VALIDATED: Mutex<CatalogCache> = Mutex::new(CatalogCache {
    entries: VecDeque::new(),
    bytes: 0,
});

pub(super) fn contains(signature: &Signature) -> bool {
    VALIDATED
        .lock()
        .is_ok_and(|cache| cache.entries.contains(signature))
}

pub(super) fn remember(signature: Signature) {
    // A poisoned cache only loses the optimization; it never grants validation.
    if let Ok(mut cache) = VALIDATED.lock() {
        cache.remember(signature);
    }
}

pub(super) fn signature(connection: &Connection) -> Result<Signature> {
    // One read snapshot includes the version, NULLs and exact definitions.
    // SQLite JSON encoding is unambiguous, including quotes/control characters.
    // Compare the whole encoding: no hash, path, mtime or version-only hit.
    let value: String = connection.query_row(
        "SELECT json_group_array(json_array(user_version,type,name,tbl_name,sql))
         FROM (SELECT v.user_version,s.type,s.name,s.tbl_name,s.sql
         FROM pragma_user_version AS v LEFT JOIN main.sqlite_schema AS s ON 1
         ORDER BY s.type,s.name)",
        [],
        |row| row.get(0),
    )?;
    Ok(value.into_boxed_str())
}

#[cfg(test)]
mod tests;
