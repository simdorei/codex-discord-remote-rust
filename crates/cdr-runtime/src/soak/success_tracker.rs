use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use thiserror::Error;

pub(super) const OUTBOX_NAMESPACE: &str = "outbox";
pub(super) const MIRROR_NAMESPACE: &str = "mirror";
pub(super) const TRACKER_BACKEND: &str = "sqlite";
const CACHE_KIB: i64 = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct TrackerRecord {
    pub duplicate: bool,
    pub duplicate_count: u64,
}

#[derive(Debug, Error)]
pub(super) enum TrackerError {
    #[error("offline success tracker database failed: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("offline success tracker lock was poisoned")]
    LockPoisoned,
    #[error("offline success tracker counter is outside the unsigned range")]
    CounterRange,
}

pub(super) struct DiskSuccessTracker {
    connection: Mutex<Connection>,
}

impl DiskSuccessTracker {
    pub fn create(path: &Path) -> Result<Self, TrackerError> {
        let connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch(&format!(
            "PRAGMA journal_mode=TRUNCATE; \
             PRAGMA synchronous=OFF; \
             PRAGMA temp_store=FILE; \
             PRAGMA cache_size=-{CACHE_KIB}; \
             PRAGMA mmap_size=0; \
             CREATE TABLE successful_messages (\
               namespace TEXT NOT NULL, logical_id TEXT NOT NULL, \
               PRIMARY KEY(namespace, logical_id)\
             ) WITHOUT ROWID; \
             CREATE TABLE tracker_stats (\
               namespace TEXT PRIMARY KEY, unique_count INTEGER NOT NULL, \
               duplicate_count INTEGER NOT NULL\
             ) WITHOUT ROWID;"
        ))?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn record(&self, namespace: &str, logical_id: &str) -> Result<TrackerRecord, TrackerError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let duplicate = transaction.execute(
            "INSERT OR IGNORE INTO successful_messages(namespace, logical_id) VALUES (?, ?)",
            params![namespace, logical_id],
        )? == 0;
        transaction.execute(
            "INSERT INTO tracker_stats(namespace, unique_count, duplicate_count) \
             VALUES (?, ?, ?) ON CONFLICT(namespace) DO UPDATE SET \
             unique_count = unique_count + excluded.unique_count, \
             duplicate_count = duplicate_count + excluded.duplicate_count",
            params![namespace, i64::from(!duplicate), i64::from(duplicate)],
        )?;
        let duplicate_count = transaction.query_row(
            "SELECT duplicate_count FROM tracker_stats WHERE namespace = ?",
            [namespace],
            |row| row.get::<_, i64>(0),
        )?;
        transaction.commit()?;
        Ok(TrackerRecord {
            duplicate,
            duplicate_count: u64::try_from(duplicate_count)
                .map_err(|_| TrackerError::CounterRange)?,
        })
    }

    pub fn cardinality(&self) -> Result<u64, TrackerError> {
        let count = self.lock()?.query_row(
            "SELECT coalesce(sum(unique_count), 0) FROM tracker_stats",
            [],
            |row| row.get::<_, i64>(0),
        )?;
        u64::try_from(count).map_err(|_| TrackerError::CounterRange)
    }

    pub fn duplicate_count(&self, namespace: &str) -> Result<u64, TrackerError> {
        Ok(self.namespace_stats(namespace)?.1)
    }

    pub fn namespace_stats(&self, namespace: &str) -> Result<(u64, u64), TrackerError> {
        let stats = self
            .lock()?
            .query_row(
                "SELECT unique_count, duplicate_count FROM tracker_stats WHERE namespace = ?",
                [namespace],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?
            .unwrap_or_default();
        Ok((
            u64::try_from(stats.0).map_err(|_| TrackerError::CounterRange)?,
            u64::try_from(stats.1).map_err(|_| TrackerError::CounterRange)?,
        ))
    }

    pub fn cache_kib(&self) -> Result<u64, TrackerError> {
        let configured = self
            .lock()?
            .pragma_query_value(None, "cache_size", |row| row.get::<_, i64>(0))?;
        configured
            .checked_abs()
            .and_then(|value| u64::try_from(value).ok())
            .ok_or(TrackerError::CounterRange)
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, TrackerError> {
        self.connection
            .lock()
            .map_err(|_| TrackerError::LockPoisoned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqlite_tracker_detects_a_b_a_globally_with_bounded_cache() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("tracker.sqlite");
        let tracker = DiskSuccessTracker::create(&path).unwrap();

        assert!(!tracker.record(OUTBOX_NAMESPACE, "a").unwrap().duplicate);
        assert!(!tracker.record(OUTBOX_NAMESPACE, "b").unwrap().duplicate);
        let repeated = tracker.record(OUTBOX_NAMESPACE, "a").unwrap();
        assert!(repeated.duplicate);
        assert_eq!(repeated.duplicate_count, 1);
        assert!(!tracker.record(MIRROR_NAMESPACE, "a").unwrap().duplicate);
        assert_eq!(tracker.namespace_stats(OUTBOX_NAMESPACE).unwrap(), (2, 1));
        assert_eq!(tracker.namespace_stats(MIRROR_NAMESPACE).unwrap(), (1, 0));
        assert_eq!(tracker.cardinality().unwrap(), 3);
        assert!(tracker.cache_kib().unwrap() <= 256);
        assert!(path.is_file());
    }
}
