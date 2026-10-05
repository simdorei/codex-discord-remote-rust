//! A short-lived, read-only snapshot. No initialization or repair from discovery.
use std::path::Path;

use rusqlite::{Connection, OpenFlags};

use super::{LATEST_STORE_SCHEMA_VERSION, STORE_BUSY_TIMEOUT, catalog_cache};
use crate::{Result, StoreError};

#[cfg(test)]
pub(crate) mod test_support;

pub(crate) struct CheckedRead {
    connection: Connection,
    uncached: Option<catalog_cache::Signature>,
}

impl CheckedRead {
    #[cfg_attr(debug_assertions, track_caller)]
    pub(crate) fn open(path: &Path) -> Result<Self> {
        #[cfg(debug_assertions)]
        let mut profile = super::open_profile::Span::new(std::panic::Location::caller());
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        connection.busy_timeout(STORE_BUSY_TIMEOUT)?;
        connection.pragma_update(None, "query_only", true)?;
        #[cfg(debug_assertions)]
        profile.mark(super::open_profile::Phase::Connection);
        #[cfg(test)]
        test_support::boundary(test_support::Boundary::Opened, &connection)?;
        connection.execute_batch("BEGIN DEFERRED")?;
        let signature = catalog_cache::signature(&connection)?;
        #[cfg(debug_assertions)]
        profile.mark(super::open_profile::Phase::Catalog);
        #[cfg(test)]
        test_support::boundary(test_support::Boundary::Catalog, &connection)?;
        let cached = catalog_cache::contains(&signature);
        #[cfg(debug_assertions)]
        profile.mark(super::open_profile::Phase::Cache);
        if !cached {
            #[cfg(debug_assertions)]
            profile.miss();
            let found = super::schema_version(&connection)?;
            if found != LATEST_STORE_SCHEMA_VERSION {
                return Err(StoreError::UnsupportedVersion {
                    found,
                    supported: LATEST_STORE_SCHEMA_VERSION,
                });
            }
            if !super::rust_extensions_current(&connection)? {
                return Err(StoreError::Integrity(
                    "metadata discovery requires an initialized current schema; no repair attempted".into(),
                ));
            }
        }
        Ok(Self {
            connection,
            uncached: (!cached).then_some(signature),
        })
    }

    pub(crate) fn connection(&self) -> &Connection {
        &self.connection
    }

    pub(crate) fn ensure_active(&self) -> Result<()> {
        if self.connection.is_autocommit() {
            return Err(StoreError::Integrity(
                "metadata read snapshot ended before publication".into(),
            ));
        }
        Ok(())
    }

    pub(crate) fn finish(self) -> Result<()> {
        #[cfg(test)]
        test_support::boundary(test_support::Boundary::BeforeCommit, &self.connection)?;
        self.connection.execute_batch("COMMIT")?;
        if let Some(signature) = self.uncached {
            catalog_cache::remember(signature);
        }
        Ok(())
    }
}
// Closing the owned connection rolls back any unfinished read transaction.
// Nothing is pooled, and no connection or transaction crosses the round's return.
