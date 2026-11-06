mod captured;
mod encoding;
pub(crate) mod handle;
mod job;
mod launch;
mod resolve;
mod stdio;

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::File;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use windows_sys::Win32::Foundation::{
    ERROR_INVALID_PARAMETER, FILETIME, GetLastError, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetExitCodeProcess, GetProcessTimes, OpenProcess,
    PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW, WaitForSingleObject,
};

use handle::OwnedHandle;

use crate::error::{NativeError, api_error};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiveProcessMetadata {
    pub pid: u32,
    pub process_identity: String,
    pub executable_path: PathBuf,
}

/// Same PID + UTC .NET creation ticks used by the Windows restart controller.
pub fn current_process_identity() -> Result<String, NativeError> {
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: the current-process pseudo handle is valid; all four outputs are
    // distinct writable FILETIMEs and no ownership of the pseudo handle is taken.
    if unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &raw mut creation,
            &raw mut exit,
            &raw mut kernel,
            &raw mut user,
        )
    } == 0
    {
        return Err(api_error("GetProcessTimes(current process)"));
    }
    Ok(format_process_identity(std::process::id(), creation))
}

/// Inspects a running OS process by PID and returns its exact `<pid>|<dotnet_ticks>` identity
/// and image path if the process is currently alive.
pub fn query_live_process_metadata(pid: u32) -> Result<Option<LiveProcessMetadata>, NativeError> {
    if pid == 0 {
        return Ok(None);
    }
    // SAFETY: querying limited information for an arbitrary process ID is valid.
    let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    let Some(handle) = OwnedHandle::new(raw) else {
        // SAFETY: GetLastError has no preconditions and is read immediately after OpenProcess.
        let code = unsafe { GetLastError() };
        if code == ERROR_INVALID_PARAMETER {
            return Ok(None);
        }
        return Err(NativeError::Api {
            operation: "OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)",
            source: std::io::Error::from_raw_os_error(code.cast_signed()),
        });
    };
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: `handle` is a live process handle with query-limited rights; outputs are writable.
    if unsafe {
        GetProcessTimes(
            handle.raw(),
            &raw mut creation,
            &raw mut exit,
            &raw mut kernel,
            &raw mut user,
        )
    } == 0
    {
        return Err(api_error("GetProcessTimes(target process)"));
    }
    if exit.dwLowDateTime != 0 || exit.dwHighDateTime != 0 {
        return Ok(None);
    }
    let mut size: u32 = 32_768;
    let mut buffer = vec![0_u16; 32_768];
    // SAFETY: `handle` is live and `buffer`/`size` are writable and bounded.
    if unsafe { QueryFullProcessImageNameW(handle.raw(), 0, buffer.as_mut_ptr(), &raw mut size) }
        == 0
    {
        return Err(api_error("QueryFullProcessImageNameW"));
    }
    let executable_path = PathBuf::from(OsString::from_wide(
        &buffer[..usize::try_from(size).unwrap_or_default()],
    ));
    Ok(Some(LiveProcessMetadata {
        pid,
        process_identity: format_process_identity(pid, creation),
        executable_path,
    }))
}

fn format_process_identity(pid: u32, creation: FILETIME) -> String {
    let windows_ticks =
        (u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime);
    let dotnet_ticks = windows_ticks + 504_911_232_000_000_000;
    format!("{pid}|{}", dotnet_ticks - dotnet_ticks % 10)
}

pub struct WindowProcess {
    process: Option<OwnedHandle>,
    job: Option<OwnedHandle>,
    process_id: u32,
}

pub struct CapturedWindowProcess {
    process: Option<OwnedHandle>,
    job: Option<OwnedHandle>,
    process_id: u32,
    stdin: Option<File>,
    stdout: Option<File>,
    stderr: Option<File>,
}

impl WindowProcess {
    pub fn launch(
        executable: &Path,
        arguments: &[String],
        cwd: &Path,
        environment: &HashMap<String, String>,
    ) -> Result<Self, NativeError> {
        launch::launch(executable, arguments, cwd, environment)
    }

    #[must_use]
    pub fn process_id(&self) -> u32 {
        self.process_id
    }

    pub fn is_running(&self) -> Result<bool, NativeError> {
        let Some(process) = &self.process else {
            return Ok(false);
        };
        // SAFETY: `process` owns a valid kernel process handle for this call.
        let result = unsafe { WaitForSingleObject(process.raw(), 0) };
        match result {
            WAIT_TIMEOUT => Ok(true),
            WAIT_OBJECT_0 => Ok(false),
            WAIT_FAILED => Err(api_error("WaitForSingleObject")),
            value => Err(NativeError::UnexpectedWait(value)),
        }
    }

    pub fn terminate_tree(&mut self, timeout: Duration) -> Result<(), NativeError> {
        let Some(job) = &self.job else {
            self.process.take();
            return Ok(());
        };
        job::terminate(job, timeout)?;
        self.job.take();
        self.process.take();
        Ok(())
    }
}

impl CapturedWindowProcess {
    pub fn launch(
        executable: &Path,
        arguments: &[String],
        cwd: &Path,
        environment: &HashMap<String, String>,
    ) -> Result<Self, NativeError> {
        captured::launch(executable, arguments, cwd, environment)
    }

    pub fn launch_piped(
        executable: &Path,
        arguments: &[String],
        cwd: &Path,
        environment: &HashMap<String, String>,
    ) -> Result<Self, NativeError> {
        captured::launch_piped(executable, arguments, cwd, environment)
    }

    #[must_use]
    pub fn process_id(&self) -> u32 {
        self.process_id
    }

    pub fn take_stdin(&mut self) -> Option<File> {
        self.stdin.take()
    }

    pub fn take_stdout(&mut self) -> Option<File> {
        self.stdout.take()
    }

    pub fn take_stderr(&mut self) -> Option<File> {
        self.stderr.take()
    }

    pub fn try_wait(&self) -> Result<Option<i32>, NativeError> {
        let Some(process) = &self.process else {
            return Ok(None);
        };
        // SAFETY: `process` owns a valid kernel process handle for this non-blocking wait.
        let result = unsafe { WaitForSingleObject(process.raw(), 0) };
        match result {
            WAIT_TIMEOUT => Ok(None),
            WAIT_OBJECT_0 => process_exit_code(process).map(Some),
            WAIT_FAILED => Err(api_error("WaitForSingleObject(captured process)")),
            value => Err(NativeError::UnexpectedWait(value)),
        }
    }

    pub fn start_kill(&self) -> Result<(), NativeError> {
        let Some(job) = &self.job else {
            return Ok(());
        };
        job::kill(job)
    }

    pub fn terminate_tree(&mut self, timeout: Duration) -> Result<(), NativeError> {
        let Some(job) = &self.job else {
            self.process.take();
            return Ok(());
        };
        job::terminate(job, timeout)?;
        self.job.take();
        self.process.take();
        Ok(())
    }
}

fn process_exit_code(process: &OwnedHandle) -> Result<i32, NativeError> {
    let mut code = 0;
    // SAFETY: `process` is signaled and owns a valid process handle; `code` is writable.
    if unsafe { GetExitCodeProcess(process.raw(), &raw mut code) } == 0 {
        return Err(api_error("GetExitCodeProcess"));
    }
    Ok(i32::from_ne_bytes(code.to_ne_bytes()))
}

impl Drop for WindowProcess {
    fn drop(&mut self) {
        self.job.take();
        self.process.take();
    }
}

impl Drop for CapturedWindowProcess {
    fn drop(&mut self) {
        self.job.take();
        self.process.take();
    }
}

#[cfg(test)]
mod identity_tests {
    #[test]
    fn native_identity_matches_the_powershell_restart_fence() {
        let output = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-Command"])
            .arg(format!("$p=Get-Process -Id {}; $t=$p.StartTime.ToUniversalTime().Ticks; $t-=$t%10; Write-Output \"$($p.Id)|$t\"", std::process::id()))
            .output().unwrap();
        assert!(output.status.success());
        assert_eq!(
            super::current_process_identity().unwrap(),
            String::from_utf8(output.stdout).unwrap().trim()
        );
    }
}
