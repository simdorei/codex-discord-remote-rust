use std::mem::size_of;
use std::time::Duration;

use windows_sys::Win32::Foundation::{WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, ResumeThread,
    TerminateProcess, WaitForSingleObject,
};

use super::handle::OwnedHandle;
use crate::error::{NativeError, api_error};

pub struct ProcessOwnership {
    pub process: OwnedHandle,
    pub job: OwnedHandle,
    pub process_id: u32,
}

pub fn adopt_created_handles(
    info: &PROCESS_INFORMATION,
) -> Result<(OwnedHandle, OwnedHandle), NativeError> {
    let process = OwnedHandle::new(info.hProcess);
    let thread = OwnedHandle::new(info.hThread);
    match (process, thread) {
        (Some(process), Some(thread)) => Ok((process, thread)),
        (Some(process), thread) => {
            stop_suspended(&process);
            drop(thread);
            Err(invalid_created_handles())
        }
        (None, thread) => {
            // SAFETY: this is defensive rollback for an inconsistent successful CreateProcessW
            // result. The reported PID came from that call and only termination/wait are asked.
            let reopened = unsafe {
                OpenProcess(PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, 0, info.dwProcessId)
            };
            if let Some(process) = OwnedHandle::new(reopened) {
                stop_suspended(&process);
            }
            drop(thread);
            Err(invalid_created_handles())
        }
    }
}

pub fn assign_and_resume(
    process: OwnedHandle,
    thread: OwnedHandle,
    process_id: u32,
) -> Result<ProcessOwnership, NativeError> {
    let job = match create_and_assign(&process) {
        Ok(job) => job,
        Err(error) => {
            stop_suspended(&process);
            return Err(error);
        }
    };
    // SAFETY: `thread` is the suspended primary thread returned by CreateProcessW.
    if unsafe { ResumeThread(thread.raw()) } == u32::MAX {
        let error = api_error("ResumeThread");
        // SAFETY: the process was assigned to this retained Job Object.
        let _ = unsafe { TerminateJobObject(job.raw(), 1) };
        return Err(error);
    }
    drop(thread);
    Ok(ProcessOwnership {
        process,
        job,
        process_id,
    })
}

pub fn terminate(job: &OwnedHandle, timeout: Duration) -> Result<(), NativeError> {
    kill(job)?;
    let milliseconds = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);
    // SAFETY: the retained Job Object handle remains valid for the duration of this wait.
    let result = unsafe { WaitForSingleObject(job.raw(), milliseconds) };
    match result {
        WAIT_OBJECT_0 => Ok(()),
        WAIT_TIMEOUT => Err(NativeError::StopTimeout),
        WAIT_FAILED => Err(api_error("WaitForSingleObject(job)")),
        value => Err(NativeError::UnexpectedWait(value)),
    }
}

pub fn kill(job: &OwnedHandle) -> Result<(), NativeError> {
    // SAFETY: `job` owns a valid Job Object handle and remains live through the wait.
    if unsafe { TerminateJobObject(job.raw(), 1) } == 0 {
        return Err(api_error("TerminateJobObject"));
    }
    Ok(())
}

fn create_and_assign(process: &OwnedHandle) -> Result<OwnedHandle, NativeError> {
    // SAFETY: null security attributes and name request an unnamed Job Object.
    let raw_job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    let job = OwnedHandle::new(raw_job).ok_or_else(|| api_error("CreateJobObjectW"))?;
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    // SAFETY: the Job Object handle and correctly sized limit structure remain live.
    if unsafe {
        SetInformationJobObject(
            job.raw(),
            JobObjectExtendedLimitInformation,
            (&raw const limits).cast(),
            u32::try_from(size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>())
                .expect("job information size"),
        )
    } == 0
    {
        return Err(api_error("SetInformationJobObject"));
    }
    // SAFETY: both retained handles are valid and the process is still suspended.
    if unsafe { AssignProcessToJobObject(job.raw(), process.raw()) } == 0 {
        return Err(api_error("AssignProcessToJobObject"));
    }
    Ok(job)
}

fn stop_suspended(process: &OwnedHandle) {
    // SAFETY: the retained process handle is valid; this is rollback before publication.
    let _ = unsafe { TerminateProcess(process.raw(), 1) };
    // SAFETY: the same handle remains valid while rollback waits briefly.
    let _ = unsafe { WaitForSingleObject(process.raw(), 5_000) };
}

fn invalid_created_handles() -> NativeError {
    NativeError::InvalidInput("CreateProcessW returned an invalid process or thread handle".into())
}
