use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::{Map, Value};
use thiserror::Error;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SavedThreadSettings {
    pub model: Option<String>,
    pub reasoning: Option<String>,
    pub speed: Option<String>,
}

#[derive(Debug, Error)]
pub enum BridgeStateError {
    #[error("could not access bridge state {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("bridge state {path} is not valid JSON: {source}")]
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("bridge state did not contain a JSON object: {0}")]
    NotObject(PathBuf),
    #[error("bridge state lock was poisoned")]
    LockPoisoned,
}

pub struct BridgeState {
    path: PathBuf,
    lock: Mutex<()>,
}

impl BridgeState {
    #[must_use]
    pub const fn new(path: PathBuf) -> Self {
        Self {
            path,
            lock: Mutex::new(()),
        }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn selected_thread_id(&self) -> Result<Option<String>, BridgeStateError> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| BridgeStateError::LockPoisoned)?;
        let state = self.load()?;
        Ok(clean_string(state.get("selected_thread_id")))
    }

    pub fn tracked_thread_ids(&self) -> Result<Vec<String>, BridgeStateError> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| BridgeStateError::LockPoisoned)?;
        let state = self.load()?;
        let mut ids = BTreeSet::new();
        if let Some(selected) = clean_string(state.get("selected_thread_id")) {
            ids.insert(selected);
        }
        if let Some(settings) = state.get("thread_settings").and_then(Value::as_object) {
            ids.extend(
                settings
                    .keys()
                    .filter(|thread_id| !thread_id.is_empty())
                    .cloned(),
            );
        }
        Ok(ids.into_iter().collect())
    }

    pub fn set_selected_thread_id(&self, thread_id: Option<&str>) -> Result<(), BridgeStateError> {
        self.update(|state| {
            if let Some(thread_id) = thread_id.map(str::trim).filter(|value| !value.is_empty()) {
                state.insert("selected_thread_id".into(), Value::String(thread_id.into()));
            } else {
                state.remove("selected_thread_id");
            }
        })
    }

    pub fn thread_settings(
        &self,
        thread_id: &str,
    ) -> Result<SavedThreadSettings, BridgeStateError> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| BridgeStateError::LockPoisoned)?;
        let state = self.load()?;
        let settings = state
            .get("thread_settings")
            .and_then(Value::as_object)
            .and_then(|all| all.get(thread_id))
            .and_then(Value::as_object);
        Ok(SavedThreadSettings {
            model: settings.and_then(|value| clean_string(value.get("model"))),
            reasoning: settings.and_then(|value| clean_string(value.get("reasoning"))),
            speed: settings.and_then(|value| clean_string(value.get("speed"))),
        })
    }

    pub fn remember_thread_settings(
        &self,
        thread_id: &str,
        model: Option<&str>,
        reasoning: Option<&str>,
        speed: Option<&str>,
    ) -> Result<(), BridgeStateError> {
        if model.is_none() && reasoning.is_none() && speed.is_none() {
            return Ok(());
        }
        self.update(|state| {
            let all = object_entry(state, "thread_settings");
            let settings = object_entry(all, thread_id);
            set_optional_string(settings, "model", model);
            set_optional_string(settings, "reasoning", reasoning);
            set_optional_string(settings, "speed", speed);
        })
    }

    /// Applies local bridge state inherited by a persistent app-server fork.
    ///
    /// The source settings remain intact because the original Codex thread still
    /// exists. A pre-existing target entry is never overwritten.
    pub fn apply_thread_fork(
        &self,
        source_thread_id: &str,
        target_thread_id: &str,
    ) -> Result<(), BridgeStateError> {
        if source_thread_id == target_thread_id {
            return Ok(());
        }
        self.update(|state| {
            if clean_string(state.get("selected_thread_id")).as_deref() == Some(source_thread_id) {
                state.insert(
                    "selected_thread_id".into(),
                    Value::String(target_thread_id.into()),
                );
            }
            let all = object_entry(state, "thread_settings");
            if !all.contains_key(target_thread_id)
                && let Some(source) = all.get(source_thread_id).cloned()
            {
                all.insert(target_thread_id.into(), source);
            }
        })
    }

    fn update(&self, mutate: impl FnOnce(&mut Map<String, Value>)) -> Result<(), BridgeStateError> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| BridgeStateError::LockPoisoned)?;
        let mut state = self.load()?;
        mutate(&mut state);
        self.save(&state)
    }

    fn load(&self) -> Result<Map<String, Value>, BridgeStateError> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Map::new()),
            Err(source) => return Err(self.io(source)),
        };
        let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes);
        let value =
            serde_json::from_slice::<Value>(bytes).map_err(|source| BridgeStateError::Json {
                path: self.path.clone(),
                source,
            })?;
        value
            .as_object()
            .cloned()
            .ok_or_else(|| BridgeStateError::NotObject(self.path.clone()))
    }

    fn save(&self, state: &Map<String, Value>) -> Result<(), BridgeStateError> {
        let parent = self.path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent).map_err(|source| self.io(source))?;
        let mut bytes =
            serde_json::to_vec_pretty(state).map_err(|source| BridgeStateError::Json {
                path: self.path.clone(),
                source,
            })?;
        bytes.push(b'\n');
        let mut temporary =
            tempfile::NamedTempFile::new_in(parent).map_err(|source| self.io(source))?;
        temporary
            .write_all(&bytes)
            .map_err(|source| self.io(source))?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|source| self.io(source))?;
        temporary
            .persist(&self.path)
            .map_err(|error| self.io(error.error))?;
        Ok(())
    }

    fn io(&self, source: std::io::Error) -> BridgeStateError {
        BridgeStateError::Io {
            path: self.path.clone(),
            source,
        }
    }
}

fn clean_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn object_entry<'a>(state: &'a mut Map<String, Value>, key: &str) -> &'a mut Map<String, Value> {
    if !state.get(key).is_some_and(Value::is_object) {
        state.insert(key.into(), Value::Object(Map::new()));
    }
    state
        .get_mut(key)
        .and_then(Value::as_object_mut)
        .expect("object inserted")
}

fn set_optional_string(state: &mut Map<String, Value>, key: &str, value: Option<&str>) {
    if let Some(value) = value {
        state.insert(key.into(), Value::String(value.into()));
    }
}
