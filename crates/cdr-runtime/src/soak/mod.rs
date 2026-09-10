use std::io;
use std::path::Path;

mod config;
mod fake_backend;
mod fake_sender;
mod fixture;
mod harness;
mod path_key;
mod recovery;
mod report;
mod runner;
mod schedule;
mod success_tracker;

pub use config::SoakConfig;
pub use report::{SoakAssertions, SoakCounters, SoakSummary, write_summary};
pub use runner::run;

pub type SoakResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub(super) fn invalid_input(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

pub(super) fn ensure_parent(path: &Path) -> SoakResult<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    Ok(())
}
