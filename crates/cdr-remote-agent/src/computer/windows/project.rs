mod actions;
mod launch;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use cdr_remote_protocol::request::ComputerApp;
use cdr_windows_native::WindowProcess;

use super::model::{platform, require_matching, resolve_project};
use crate::computer::{ComputerError, ComputerIdentity};

const STOP_TIMEOUT: Duration = Duration::from_secs(5);

struct OwnedApplication {
    app: ComputerApp,
    process: WindowProcess,
    window_process_id: u32,
    process_path: String,
    profile: Option<PathBuf>,
}

pub(super) struct WindowsProjectPlatform {
    owned: Mutex<HashMap<u64, OwnedApplication>>,
}

impl WindowsProjectPlatform {
    pub fn new() -> Self {
        Self {
            owned: Mutex::new(HashMap::new()),
        }
    }

    fn lock(&self) -> Result<MutexGuard<'_, HashMap<u64, OwnedApplication>>, ComputerError> {
        self.owned.lock().map_err(|_| ComputerError::State)
    }

    fn resolve_owned(&self, window_id: u64) -> Result<super::model::ResolvedWindow, ComputerError> {
        let (pid, path, running) = {
            let owned = self.lock()?;
            let app = owned.get(&window_id).ok_or_else(not_owned)?;
            (
                app.window_process_id,
                app.process_path.clone(),
                app.process.is_running().map_err(|error| platform(&error))?,
            )
        };
        if !running {
            return Err(not_owned());
        }
        let current = resolve_project(window_id)?;
        if current.identity.process_id != pid
            || !current.identity.process_path.eq_ignore_ascii_case(&path)
        {
            return Err(platform(&"The launched window identity changed."));
        }
        Ok(current)
    }

    fn current(
        &self,
        identity: &ComputerIdentity,
        include_title: bool,
    ) -> Result<(), ComputerError> {
        require_matching(
            identity,
            &self.resolve_owned(identity.window_id)?,
            include_title,
        )
    }

    fn prune(&self) -> Result<(), ComputerError> {
        let exited = self
            .lock()?
            .iter()
            .filter_map(|(id, app)| match app.process.is_running() {
                Ok(false) => Some(Ok(*id)),
                Ok(true) => None,
                Err(error) => Some(Err(platform(&error))),
            })
            .collect::<Result<Vec<_>, _>>()?;
        for id in exited {
            let profile = self.lock()?.get(&id).and_then(|app| app.profile.clone());
            launch::cleanup_profile(profile.as_deref())?;
            self.lock()?.remove(&id);
        }
        Ok(())
    }

    fn stop_owned(&self) -> Result<(), ComputerError> {
        loop {
            let Some(id) = self.lock()?.keys().next().copied() else {
                return Ok(());
            };
            let profile = {
                let mut owned = self.lock()?;
                let app = owned.get_mut(&id).ok_or_else(not_owned)?;
                app.process
                    .terminate_tree(STOP_TIMEOUT)
                    .map_err(|error| platform(&error))?;
                app.profile.clone()
            };
            launch::cleanup_profile(profile.as_deref())?;
            self.lock()?.remove(&id);
        }
    }
}

fn not_owned() -> ComputerError {
    platform(&"Only a window launched by this session can be controlled.")
}
