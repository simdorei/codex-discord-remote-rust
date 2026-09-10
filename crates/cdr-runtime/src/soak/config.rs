use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::time::Duration;

use super::path_key::same_normalized_absolute;
use super::{SoakResult, invalid_input};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SoakConfig {
    pub duration_secs: u64,
    pub seed: u64,
    pub output: PathBuf,
    pub events: PathBuf,
    pub test_fail_after_cycles: Option<u64>,
}

impl SoakConfig {
    pub fn from_env() -> SoakResult<Self> {
        Self::parse(std::env::args_os().skip(1))
    }

    pub fn parse(args: impl IntoIterator<Item = OsString>) -> SoakResult<Self> {
        let mut duration_secs = None;
        let mut seed = None;
        let mut output = None;
        let mut events = None;
        let mut test_fail_after_cycles = None;
        let mut args = args.into_iter();
        while let Some(flag) = args.next() {
            let value = args.next().ok_or_else(|| {
                invalid_input(format!("missing value for {}", flag.to_string_lossy()))
            })?;
            match flag.to_str() {
                Some("--duration-secs") => duration_secs = Some(parse_u64(&value, "duration")?),
                Some("--seed") => seed = Some(parse_u64(&value, "seed")?),
                Some("--output") => output = Some(PathBuf::from(value)),
                Some("--events") => events = Some(PathBuf::from(value)),
                Some("--test-fail-after-cycles") => {
                    test_fail_after_cycles = Some(parse_u64(&value, "test failure cycle")?);
                }
                _ => {
                    return Err(invalid_input(format!(
                        "unknown argument: {}",
                        flag.to_string_lossy()
                    ))
                    .into());
                }
            }
        }
        let duration_secs =
            duration_secs.ok_or_else(|| invalid_input("--duration-secs is required"))?;
        if duration_secs == 0 {
            return Err(invalid_input("--duration-secs must be at least 1").into());
        }
        let output = output.ok_or_else(|| invalid_input("--output is required"))?;
        let events = events.unwrap_or_else(|| output.with_extension("events.jsonl"));
        reject_existing(&output, "--output")?;
        reject_existing(&events, "--events")?;
        if same_normalized_absolute(&output, &events)? {
            return Err(invalid_input("--output and --events must be different paths").into());
        }
        Ok(Self {
            duration_secs,
            seed: seed.ok_or_else(|| invalid_input("--seed is required"))?,
            output,
            events,
            test_fail_after_cycles,
        })
    }

    #[must_use]
    pub const fn duration(&self) -> Duration {
        Duration::from_secs(self.duration_secs)
    }
}

fn reject_existing(path: &std::path::Path, label: &str) -> SoakResult<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(invalid_input(format!(
            "{label} already exists; offline soak evidence is never overwritten"
        ))
        .into()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn parse_u64(value: &OsString, label: &str) -> SoakResult<u64> {
    Ok(value
        .to_str()
        .ok_or_else(|| invalid_input(format!("{label} must be UTF-8")))?
        .parse()
        .map_err(|_| invalid_input(format!("{label} must be an unsigned integer")))?)
}
