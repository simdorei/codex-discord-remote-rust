use std::path::Path;

use cdr_remote_protocol::output::ComputerWindowEntry;
use cdr_windows_native::{DeviceWindowInfo, enumerate_visible_windows, inspect_visible_window};
use sha2::{Digest, Sha256};

use super::policy::{ProjectApplication, project_application, safe_project_title};
use crate::computer::{ComputerError, ComputerIdentity};

#[derive(Clone)]
pub(super) struct ResolvedWindow {
    pub entry: ComputerWindowEntry,
    pub identity: ComputerIdentity,
}

pub(super) fn list_device_windows() -> Result<Vec<ComputerWindowEntry>, ComputerError> {
    enumerate_visible_windows()
        .map_err(|error| platform(&error))?
        .into_iter()
        .map(|value| resolve(&value, None).map(|resolved| resolved.entry))
        .collect()
}

pub(super) fn resolve_device(window_id: u64) -> Result<ResolvedWindow, ComputerError> {
    resolve(
        &inspect_visible_window(window_id).map_err(|error| platform(&error))?,
        None,
    )
}

pub(super) fn resolve_project(window_id: u64) -> Result<ResolvedWindow, ComputerError> {
    let native = inspect_visible_window(window_id).map_err(|error| platform(&error))?;
    let application = project_application(Path::new(&native.process_path))?;
    resolve(&native, Some(application))
}

pub(super) fn same_window(expected: &ComputerIdentity, current: &ComputerIdentity) -> bool {
    same_frame(expected, current) && expected.title_digest == current.title_digest
}

pub(super) fn same_frame(expected: &ComputerIdentity, current: &ComputerIdentity) -> bool {
    expected.window_id == current.window_id
        && expected.process_id == current.process_id
        && expected
            .process_path
            .eq_ignore_ascii_case(&current.process_path)
        && expected.left == current.left
        && expected.top == current.top
        && expected.width == current.width
        && expected.height == current.height
}

pub(super) fn change_summary(expected: &ComputerIdentity, current: &ComputerIdentity) -> String {
    format!(
        "process_id={}, process_path={}, title={}, bounds={}",
        expected.process_id == current.process_id,
        expected
            .process_path
            .eq_ignore_ascii_case(&current.process_path),
        expected.title_digest == current.title_digest,
        expected.left == current.left
            && expected.top == current.top
            && expected.width == current.width
            && expected.height == current.height,
    )
}

pub(super) fn require_matching(
    expected: &ComputerIdentity,
    current: &ResolvedWindow,
    include_title: bool,
) -> Result<(), ComputerError> {
    let matches = if include_title {
        same_window(expected, &current.identity)
    } else {
        same_frame(expected, &current.identity)
    };
    if !matches {
        return Err(ComputerError::Platform(
            "The window changed after the screenshot. Take a fresh screenshot.".into(),
        ));
    }
    if !current.entry.active {
        return Err(ComputerError::Platform(
            "The active window changed. Take a fresh screenshot before continuing.".into(),
        ));
    }
    Ok(())
}

pub(super) fn screen_point(
    identity: &ComputerIdentity,
    point: (u32, u32),
) -> Result<(i32, i32), ComputerError> {
    let x = identity.left + i64::from(point.0);
    let y = identity.top + i64::from(point.1);
    Ok((
        i32::try_from(x).map_err(|_| bounds())?,
        i32::try_from(y).map_err(|_| bounds())?,
    ))
}

pub(super) fn platform(error: &impl ToString) -> ComputerError {
    ComputerError::Platform(error.to_string())
}

fn resolve(
    native: &DeviceWindowInfo,
    application: Option<ProjectApplication>,
) -> Result<ResolvedWindow, ComputerError> {
    let title = match application {
        Some(app) => safe_project_title(app, &native.title)?,
        None => normalize_title(&native.title),
    };
    let process_name = application.map_or_else(
        || {
            Path::new(&native.process_path)
                .file_name()
                .map_or_else(String::new, |value| value.to_string_lossy().into_owned())
        },
        |app| app.process_name().into(),
    );
    let identity = ComputerIdentity {
        window_id: native.window_id,
        process_id: native.process_id,
        process_path: native.process_path.to_lowercase(),
        title_digest: hex::encode(Sha256::digest(native.title.as_bytes())),
        left: i64::from(native.rect.left),
        top: i64::from(native.rect.top),
        width: u64::from(native.rect.width),
        height: u64::from(native.rect.height),
    };
    Ok(ResolvedWindow {
        entry: ComputerWindowEntry {
            window_id: native.window_id,
            title: title.chars().take(512).collect(),
            process_name,
            left: identity.left,
            top: identity.top,
            width: identity.width,
            height: identity.height,
            active: native.active,
        },
        identity,
    })
}

fn normalize_title(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn bounds() -> ComputerError {
    ComputerError::Platform("window coordinates exceed the Windows screen range".into())
}
