use std::time::Duration;

/// Cold desktop app-server initialization includes plugin and account discovery.
pub const APP_SERVER_INITIALIZE_TIMEOUT: Duration = Duration::from_secs(30);

/// Enclosing startup deadlines must also allow process setup and failed-start cleanup.
pub const APP_SERVER_STARTUP_TIMEOUT: Duration = Duration::from_secs(45);
