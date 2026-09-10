mod captured;
mod encoding;
pub(crate) mod handle;
mod job;
mod launch;
mod resolve;
mod stdio;

use std::collections::HashMap;
use std::fs::File;
use std::path::Path;
use std::time::Duration;

use windows_sys::Win32::Foundation::{WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows_sys::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};

use handle::OwnedHandle;

use crate::error::{NativeError, api_error};

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
