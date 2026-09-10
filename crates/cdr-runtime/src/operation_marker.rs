use std::path::Path;

const STOP_MARKER: &str = ".codex_discord_rust.stop";
const RESTART_MARKER: &str = ".codex_discord_rust.restart";

#[must_use]
pub fn shutdown_requested(root: &Path) -> bool {
    root.join(STOP_MARKER).is_file() || root.join(RESTART_MARKER).is_file()
}

#[must_use]
pub fn stop_requested(root: &Path) -> bool {
    root.join(STOP_MARKER).is_file()
}

#[must_use]
pub fn restart_requested(root: &Path) -> bool {
    !root.join(STOP_MARKER).is_file() && root.join(RESTART_MARKER).is_file()
}
