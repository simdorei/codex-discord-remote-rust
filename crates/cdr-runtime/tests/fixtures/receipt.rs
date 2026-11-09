use std::os::windows::fs::OpenOptionsExt;

pub fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let root = std::env::current_dir()?;
    let _lock = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .share_mode(0)
        .open(root.join("receipt-worker-lock"))?;
    std::fs::write(
        root.join("receipt-worker.json"),
        serde_json::to_vec(&serde_json::json!({
            "pid":std::process::id(),
            "identity":cdr_windows_native::current_process_identity()?,
        }))?,
    )?;
    // A controlled stand-in for a filesystem call that never returns in the receipt worker.
    std::thread::sleep(std::time::Duration::from_mins(1));
    Err("receipt worker should have been terminated before this point".into())
}
