mod clients;
mod model;
mod schema;
mod token_sql;
mod tokens;

use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub use model::{OAuthStoreError, OAuthStoreLimits, OAuthTokenRecord, RefreshRotationOutcome};
use rusqlite::Connection;

pub struct OAuthStore {
    connection: Mutex<Connection>,
    limits: OAuthStoreLimits,
}

impl OAuthStore {
    pub fn open(path: &Path, limits: OAuthStoreLimits) -> Result<Self, OAuthStoreError> {
        let limits = limits.validate()?;
        if path != Path::new(":memory:")
            && let Some(parent) = path.parent()
        {
            std::fs::create_dir_all(parent).map_err(|source| OAuthStoreError::CreateDirectory {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        schema::initialize(&connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
            limits,
        })
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>, OAuthStoreError> {
        self.connection
            .lock()
            .map_err(|_| OAuthStoreError::LockPoisoned)
    }
}

fn unix_timestamp() -> Result<i64, OAuthStoreError> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| OAuthStoreError::InvalidSystemClock)?
        .as_secs();
    i64::try_from(seconds).map_err(|_| OAuthStoreError::InvalidSystemClock)
}
