#![cfg(windows)]

const STARTUP: &str = include_str!("../src/client/startup.rs");
const PROCESS: &str = include_str!("../src/process.rs");
const NATIVE_CAPTURED: &str = include_str!("../../cdr-windows-native/src/process/captured.rs");

#[test]
fn app_server_uses_suspended_job_assignment_before_resume() {
    assert!(PROCESS.contains("CapturedWindowProcess::launch_piped"));
    assert!(STARTUP.contains("spawn_app_server(config)?"));
    assert!(!STARTUP.contains("ProcessTreeGuard::attach_process_id"));

    let creation = NATIVE_CAPTURED
        .find("let created = unsafe")
        .expect("Windows process creation must be explicit");
    let suspended = creation
        + NATIVE_CAPTURED[creation..]
            .find("CREATE_SUSPENDED")
            .expect("Windows launch must begin suspended");
    let assigned = NATIVE_CAPTURED
        .find("let ownership = assign_and_resume")
        .expect("Windows launch must assign its Job Object");
    let returned = NATIVE_CAPTURED
        .find("Ok((ownership, stdio))")
        .expect("assigned process must be returned");
    assert!(creation < suspended);
    assert!(suspended < assigned);
    assert!(assigned < returned);
}
