use std::collections::HashMap;
use std::path::Path;

use windows_sys::Win32::System::Threading::{
    CREATE_NEW_CONSOLE, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW,
    PROCESS_INFORMATION, STARTUPINFOW,
};

use super::WindowProcess;
use super::encoding::{command_line, environment_block, wide};
use super::job::{adopt_created_handles, assign_and_resume};
use super::resolve;
use crate::error::{NativeError, api_error};

pub fn launch(
    executable: &Path,
    arguments: &[String],
    cwd: &Path,
    environment: &HashMap<String, String>,
) -> Result<WindowProcess, NativeError> {
    let executable_path = resolve::executable(executable, environment)?;
    let application = wide(executable_path.as_os_str(), "executable")?;
    let mut command = command_line(executable.as_os_str(), arguments)?;
    let cwd = wide(cwd.as_os_str(), "working directory")?;
    let environment = environment_block(environment)?;
    let startup = STARTUPINFOW {
        cb: u32::try_from(size_of::<STARTUPINFOW>()).expect("STARTUPINFOW size"),
        ..STARTUPINFOW::default()
    };
    let mut info = PROCESS_INFORMATION::default();
    // SAFETY: every pointer targets a live, NUL-terminated buffer; output structs are valid and
    // handles are immediately adopted below. Handle inheritance is disabled.
    let created = unsafe {
        CreateProcessW(
            application.as_ptr(),
            command.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            CREATE_NEW_CONSOLE | CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT,
            environment.as_ptr().cast(),
            cwd.as_ptr(),
            &raw const startup,
            &raw mut info,
        )
    };
    if created == 0 {
        return Err(api_error("CreateProcessW"));
    }
    let (process, thread) = adopt_created_handles(&info)?;
    let ownership = assign_and_resume(process, thread, info.dwProcessId)?;
    Ok(WindowProcess {
        process: Some(ownership.process),
        job: Some(ownership.job),
        process_id: ownership.process_id,
    })
}
