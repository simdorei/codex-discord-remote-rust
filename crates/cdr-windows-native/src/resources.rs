//! Read-only host resource measurements; byte units and CPU scope stay explicit.
use crate::NativeError;
use crate::error::api_error;
use std::{os::windows::ffi::OsStrExt, path::Path};
use windows_sys::Win32::{
    Foundation::FILETIME,
    Storage::FileSystem::GetDiskFreeSpaceExW,
    System::{
        SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX},
        Threading::{GetActiveProcessorCount, GetSystemTimes},
    },
};

#[derive(Clone, Copy, Debug)]
pub struct CpuTimes {
    pub idle: u64,
    pub kernel: u64,
    pub user: u64,
}

impl CpuTimes {
    /// Windows kernel time includes idle. No interval means no measurement.
    pub fn busy_percent_since(self, before: Self) -> Result<f64, NativeError> {
        let invalid =
            || NativeError::InvalidInput("CPU sample interval is empty or inconsistent".into());
        let idle = self.idle.checked_sub(before.idle).ok_or_else(invalid)?;
        let kernel = self.kernel.checked_sub(before.kernel).ok_or_else(invalid)?;
        let user = self.user.checked_sub(before.user).ok_or_else(invalid)?;
        let total = kernel
            .checked_add(user)
            .filter(|value| *value > 0)
            .ok_or_else(invalid)?;
        let busy = total.checked_sub(idle).ok_or_else(invalid)?;
        // Ratio is bounded, so calculate basis points in integer space first.
        let basis_points =
            u32::try_from(u128::from(busy) * 10_000 / u128::from(total)).map_err(|_| invalid())?;
        Ok(f64::from(basis_points) / 100.0)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct MemoryBytes {
    pub total: u64,
    pub available: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct DiskBytes {
    pub caller_total: u64,
    pub caller_available: u64,
    pub volume_free: u64,
}

/// On machines with more than 64 processors, this covers the primary group.
pub fn cpu_times() -> Result<CpuTimes, NativeError> {
    let mut idle = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: All three output pointers refer to initialized writable FILETIMEs.
    if unsafe { GetSystemTimes(&raw mut idle, &raw mut kernel, &raw mut user) } == 0 {
        return Err(api_error("GetSystemTimes"));
    }
    let ticks =
        |time: FILETIME| u64::from(time.dwHighDateTime) << 32 | u64::from(time.dwLowDateTime);
    Ok(CpuTimes {
        idle: ticks(idle),
        kernel: ticks(kernel),
        user: ticks(user),
    })
}

pub fn active_processor_count() -> Result<u32, NativeError> {
    // SAFETY: 0xffff is ALL_PROCESSOR_GROUPS; this API takes no pointers.
    let count = unsafe { GetActiveProcessorCount(0xffff) };
    if count == 0 {
        return Err(api_error("GetActiveProcessorCount"));
    }
    Ok(count)
}

pub fn memory_bytes() -> Result<MemoryBytes, NativeError> {
    let mut status = MEMORYSTATUSEX {
        dwLength: u32::try_from(std::mem::size_of::<MEMORYSTATUSEX>())
            .map_err(|_| NativeError::InvalidInput("MEMORYSTATUSEX size exceeds u32".into()))?,
        ..Default::default()
    };
    // SAFETY: status is writable and its size field describes the exact layout.
    if unsafe { GlobalMemoryStatusEx(&raw mut status) } == 0 {
        return Err(api_error("GlobalMemoryStatusEx"));
    }
    if status.ullTotalPhys == 0 || status.ullAvailPhys > status.ullTotalPhys {
        return Err(NativeError::InvalidInput(
            "Windows returned inconsistent physical memory counters".into(),
        ));
    }
    Ok(MemoryBytes {
        total: status.ullTotalPhys,
        available: status.ullAvailPhys,
    })
}

pub fn disk_bytes(path: &Path) -> Result<DiskBytes, NativeError> {
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return Err(NativeError::InteriorNul("disk path"));
    }
    wide.push(0);
    let mut result = DiskBytes {
        caller_total: 0,
        caller_available: 0,
        volume_free: 0,
    };
    // SAFETY: wide is NUL-terminated and alive for this synchronous call; all
    // output pointers refer to distinct initialized writable u64 values.
    if unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &raw mut result.caller_available,
            &raw mut result.caller_total,
            &raw mut result.volume_free,
        )
    } == 0
    {
        return Err(api_error("GetDiskFreeSpaceExW"));
    }
    if result.caller_total == 0 || result.caller_available > result.caller_total {
        return Err(NativeError::InvalidInput(
            "Windows returned inconsistent caller disk counters".into(),
        ));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::CpuTimes;

    #[test]
    fn cpu_subtracts_idle_from_kernel_plus_user() {
        let before = CpuTimes {
            idle: 100,
            kernel: 200,
            user: 50,
        };
        let after = CpuTimes {
            idle: 130,
            kernel: 250,
            user: 100,
        };
        assert!((after.busy_percent_since(before).unwrap() - 70.0).abs() < 0.001);
    }

    #[test]
    fn invalid_or_empty_intervals_are_unknown_not_zero() {
        let before = CpuTimes {
            idle: 100,
            kernel: 200,
            user: 50,
        };
        assert!(before.busy_percent_since(before).is_err());
        for after in [
            CpuTimes {
                idle: 99,
                kernel: 250,
                user: 100,
            },
            CpuTimes {
                idle: 101,
                kernel: 199,
                user: 100,
            },
            CpuTimes {
                idle: 101,
                kernel: 250,
                user: 49,
            },
            CpuTimes {
                idle: 300,
                kernel: 250,
                user: 100,
            },
        ] {
            assert!(after.busy_percent_since(before).is_err());
        }
    }
}
