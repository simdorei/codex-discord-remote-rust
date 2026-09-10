use std::collections::HashMap;
use std::path::Path;

use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW,
    EXTENDED_STARTUPINFO_PRESENT, PROCESS_INFORMATION, STARTUPINFOW,
};

use super::CapturedWindowProcess;
use super::encoding::{command_line, environment_block, wide};
use super::job::{ProcessOwnership, adopt_created_handles, assign_and_resume};
use super::resolve;
use super::stdio::CapturedStdio;
use crate::error::{NativeError, api_error};

pub fn launch(
    executable: &Path,
    arguments: &[String],
    cwd: &Path,
    environment: &HashMap<String, String>,
) -> Result<CapturedWindowProcess, NativeError> {
    let (ownership, pipe_set) = launch_suspended(
        executable,
        arguments,
        cwd,
        environment,
        CapturedStdio::new()?,
    )?;
    let (stdout, stderr) = pipe_set.into_parent_files();
    Ok(CapturedWindowProcess {
        process: Some(ownership.process),
        job: Some(ownership.job),
        process_id: ownership.process_id,
        stdin: None,
        stdout: Some(stdout),
        stderr: Some(stderr),
    })
}

pub fn launch_piped(
    executable: &Path,
    arguments: &[String],
    cwd: &Path,
    environment: &HashMap<String, String>,
) -> Result<CapturedWindowProcess, NativeError> {
    let (ownership, pipe_set) = launch_suspended(
        executable,
        arguments,
        cwd,
        environment,
        CapturedStdio::new_piped()?,
    )?;
    let (stdin, stdout, stderr) = pipe_set.into_piped_parent_files();
    Ok(CapturedWindowProcess {
        process: Some(ownership.process),
        job: Some(ownership.job),
        process_id: ownership.process_id,
        stdin: Some(stdin),
        stdout: Some(stdout),
        stderr: Some(stderr),
    })
}

fn launch_suspended(
    executable: &Path,
    arguments: &[String],
    cwd: &Path,
    environment: &HashMap<String, String>,
    stdio: CapturedStdio,
) -> Result<(ProcessOwnership, CapturedStdio), NativeError> {
    let executable_path = resolve::executable(executable, environment)?;
    let application = wide(executable_path.as_os_str(), "executable")?;
    let mut command = command_line(executable.as_os_str(), arguments)?;
    let cwd = wide(cwd.as_os_str(), "working directory")?;
    let environment = environment_block(environment)?;
    debug_assert_eq!(stdio.inherited_count(), 3);
    let startup = stdio.startup_info();
    let mut info = PROCESS_INFORMATION::default();
    // SAFETY: input buffers, the extended startup structure, its attribute list, and inherited
    // handles remain live through this call. Only the three listed standard handles are inherited.
    let created = unsafe {
        CreateProcessW(
            application.as_ptr(),
            command.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
            CREATE_NO_WINDOW
                | CREATE_SUSPENDED
                | CREATE_UNICODE_ENVIRONMENT
                | EXTENDED_STARTUPINFO_PRESENT,
            environment.as_ptr().cast(),
            cwd.as_ptr(),
            (&raw const startup).cast::<STARTUPINFOW>(),
            &raw mut info,
        )
    };
    if created == 0 {
        return Err(api_error("CreateProcessW(captured)"));
    }
    let (process, thread) = adopt_created_handles(&info)?;
    let ownership = assign_and_resume(process, thread, info.dwProcessId)?;
    Ok((ownership, stdio))
}
