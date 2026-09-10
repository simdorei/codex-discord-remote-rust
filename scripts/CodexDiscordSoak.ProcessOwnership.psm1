Set-StrictMode -Version Latest

if ($null -eq ('CodexDiscordSoak.ProcessOwnershipNative' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;

namespace CodexDiscordSoak {
    [StructLayout(LayoutKind.Sequential)]
    internal struct BasicJobLimits {
        public long PerProcessUserTimeLimit;
        public long PerJobUserTimeLimit;
        public uint LimitFlags;
        public UIntPtr MinimumWorkingSetSize;
        public UIntPtr MaximumWorkingSetSize;
        public uint ActiveProcessLimit;
        public UIntPtr Affinity;
        public uint PriorityClass;
        public uint SchedulingClass;
    }

    [StructLayout(LayoutKind.Sequential)]
    internal struct IoCounters {
        public ulong ReadOperationCount;
        public ulong WriteOperationCount;
        public ulong OtherOperationCount;
        public ulong ReadTransferCount;
        public ulong WriteTransferCount;
        public ulong OtherTransferCount;
    }

    [StructLayout(LayoutKind.Sequential)]
    internal struct ExtendedJobLimits {
        public BasicJobLimits BasicLimitInformation;
        public IoCounters IoInfo;
        public UIntPtr ProcessMemoryLimit;
        public UIntPtr JobMemoryLimit;
        public UIntPtr PeakProcessMemoryUsed;
        public UIntPtr PeakJobMemoryUsed;
    }

    [StructLayout(LayoutKind.Sequential)]
    internal struct FileTime {
        public uint Low;
        public uint High;
    }

    public static class ProcessOwnershipNative {
        const uint KillOnJobClose = 0x00002000;
        const uint DuplicateSameAccess = 0x00000002;

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        static extern SafeFileHandle CreateJobObjectW(IntPtr attributes, string name);
        [DllImport("kernel32.dll", SetLastError = true)]
        static extern bool SetInformationJobObject(SafeFileHandle job, int infoClass,
            ref ExtendedJobLimits info, uint length);
        [DllImport("kernel32.dll", SetLastError = true)]
        static extern bool AssignProcessToJobObject(SafeFileHandle job, SafeWaitHandle process);
        [DllImport("kernel32.dll", SetLastError = true)]
        static extern bool TerminateJobObject(SafeFileHandle job, uint exitCode);
        [DllImport("kernel32.dll", SetLastError = true)]
        static extern bool TerminateProcess(SafeWaitHandle process, uint exitCode);
        [DllImport("kernel32.dll", SetLastError = true)]
        static extern uint WaitForSingleObject(SafeWaitHandle handle, uint milliseconds);
        [DllImport("kernel32.dll", SetLastError = true)]
        static extern uint GetProcessId(SafeWaitHandle process);
        [DllImport("kernel32.dll", SetLastError = true)]
        static extern bool GetProcessTimes(SafeWaitHandle process, out FileTime creation,
            out FileTime exit, out FileTime kernel, out FileTime user);
        [DllImport("kernel32.dll")]
        static extern IntPtr GetCurrentProcess();
        [DllImport("kernel32.dll", SetLastError = true)]
        static extern bool DuplicateHandle(IntPtr sourceProcess, IntPtr sourceHandle,
            IntPtr targetProcess, out SafeWaitHandle targetHandle, uint access,
            bool inheritHandle, uint options);

        static void ThrowLastError(string operation) {
            throw new Win32Exception(Marshal.GetLastWin32Error(), operation);
        }

        public static SafeFileHandle CreateKillOnCloseJob() {
            SafeFileHandle job = CreateJobObjectW(IntPtr.Zero, null);
            if (job.IsInvalid) ThrowLastError("CreateJobObjectW failed");
            ExtendedJobLimits info = new ExtendedJobLimits();
            info.BasicLimitInformation.LimitFlags = KillOnJobClose;
            if (!SetInformationJobObject(job, 9, ref info,
                    (uint)Marshal.SizeOf(typeof(ExtendedJobLimits)))) {
                job.Dispose();
                ThrowLastError("SetInformationJobObject failed");
            }
            return job;
        }

        public static SafeWaitHandle DuplicateProcessHandle(IntPtr sourceHandle) {
            SafeWaitHandle duplicate;
            IntPtr current = GetCurrentProcess();
            if (!DuplicateHandle(current, sourceHandle, current, out duplicate, 0,
                    false, DuplicateSameAccess)) ThrowLastError("DuplicateHandle failed");
            return duplicate;
        }

        public static void Assign(SafeFileHandle job, SafeWaitHandle process) {
            if (!AssignProcessToJobObject(job, process)) ThrowLastError("AssignProcessToJobObject failed");
        }
        public static void TerminateJob(SafeFileHandle job) {
            if (!TerminateJobObject(job, 1)) ThrowLastError("TerminateJobObject failed");
        }
        public static void TerminateExactProcess(SafeWaitHandle process) {
            if (!TerminateProcess(process, 1)) ThrowLastError("TerminateProcess failed");
        }
        public static uint WaitExactProcess(SafeWaitHandle process, uint milliseconds) {
            uint result = WaitForSingleObject(process, milliseconds);
            if (result == 0xffffffff) ThrowLastError("WaitForSingleObject failed");
            return result;
        }
        public static uint ExactProcessId(SafeWaitHandle process) {
            uint pid = GetProcessId(process);
            if (pid == 0) ThrowLastError("GetProcessId failed");
            return pid;
        }
        public static long ExactProcessStartTicks(SafeWaitHandle process) {
            FileTime creation, exit, kernel, user;
            if (!GetProcessTimes(process, out creation, out exit, out kernel, out user))
                ThrowLastError("GetProcessTimes failed");
            long fileTime = ((long)creation.High << 32) | creation.Low;
            return DateTime.FromFileTimeUtc(fileTime).Ticks;
        }
    }
}
'@
}

function New-CodexSoakProcessOwner {
    [pscustomobject]@{
        JobHandle = [CodexDiscordSoak.ProcessOwnershipNative]::CreateKillOnCloseJob()
        ProcessHandle = $null; Process = $null; Pid = $null; StartTicks = $null
        JobAssigned = $false; ExitConfirmed = $false
    }
}

function Register-CodexSoakOwnedProcess {
    param([Parameter(Mandatory)][object]$Owner, [Parameter(Mandatory)][Diagnostics.Process]$Process)
    $Owner.Process = $Process
    try {
        $Owner.ProcessHandle = [CodexDiscordSoak.ProcessOwnershipNative]::DuplicateProcessHandle($Process.Handle)
        $Owner.Pid = [int][CodexDiscordSoak.ProcessOwnershipNative]::ExactProcessId($Owner.ProcessHandle)
        $Owner.StartTicks = [long][CodexDiscordSoak.ProcessOwnershipNative]::ExactProcessStartTicks($Owner.ProcessHandle)
        [CodexDiscordSoak.ProcessOwnershipNative]::Assign($Owner.JobHandle, $Owner.ProcessHandle)
        $Owner.JobAssigned = $true
        [pscustomobject]@{ pid = $Owner.Pid; start_ticks = $Owner.StartTicks }
    } catch {
        $registrationFailure = $_.Exception
        $rollback = [Collections.Generic.List[string]]::new()
        if ($null -ne $Owner.ProcessHandle) {
            try {
                $stopped = Stop-CodexSoakOwnedProcess $Owner $Owner.Pid $Owner.StartTicks 5000
                foreach ($cleanupError in @($stopped.cleanup_errors)) { $rollback.Add([string]$cleanupError) }
            } catch { $rollback.Add($_.Exception.Message) }
        }
        if (-not $Owner.ExitConfirmed) {
            try { if (-not $Process.HasExited) { $Process.Kill() } } catch { $rollback.Add($_.Exception.Message) }
            while (-not $Owner.ExitConfirmed) {
                try { $Process.WaitForExit(); $Owner.ExitConfirmed = $true }
                catch { if (-not $rollback.Contains($_.Exception.Message)) { $rollback.Add($_.Exception.Message) }; Start-Sleep -Seconds 1 }
            }
        }
        $registrationFailure.Data['CodexSoakCleanupErrors'] = [object[]]@($rollback)
        throw $registrationFailure
    }
}

function Stop-CodexSoakOwnedProcess {
    param([Parameter(Mandatory)][object]$Owner, [AllowNull()][object]$ExpectedPid,
        [AllowNull()][object]$ExpectedStartTicks, [ValidateRange(1, 60000)][int]$TimeoutMilliseconds = 5000)
    $errors = [Collections.Generic.List[string]]::new(); $wait = [uint32]258
    if ($Owner.Pid -ne $ExpectedPid -or $Owner.StartTicks -ne $ExpectedStartTicks) {
        $errors.Add('Requested PID/start did not match retained exact process identity')
    }
    if ($Owner.ExitConfirmed) { $wait = 0 }
    elseif ($null -eq $Owner.ProcessHandle) {
        $errors.Add('Owned soak child had no duplicated native process handle')
        try { if (-not $Owner.Process.HasExited) { $Owner.Process.Kill() } } catch { $errors.Add($_.Exception.Message) }
    } else {
        try { $wait = [CodexDiscordSoak.ProcessOwnershipNative]::WaitExactProcess($Owner.ProcessHandle, 0) }
        catch { $errors.Add("initial_exact_wait_failed: $($_.Exception.Message)") }
    }
    if ($wait -ne 0 -and $null -ne $Owner.ProcessHandle) {
        try { $jobUsable = $null -ne $Owner.JobHandle -and -not $Owner.JobHandle.IsClosed -and -not $Owner.JobHandle.IsInvalid }
        catch { $jobUsable = $false; $errors.Add("job_handle_check_failed: $($_.Exception.Message)") }
        if ($jobUsable) {
            try { [CodexDiscordSoak.ProcessOwnershipNative]::TerminateJob($Owner.JobHandle) }
            catch { $errors.Add("job_terminate_failed: $($_.Exception.Message)") }
            finally { try { $Owner.JobHandle.Dispose() } catch { $errors.Add("job_close_failed: $($_.Exception.Message)") }; $Owner.JobHandle = $null }
        } else { $errors.Add('Owned process Job handle was unavailable during termination') }
        if (-not $Owner.JobAssigned -or -not $jobUsable) {
            try { [CodexDiscordSoak.ProcessOwnershipNative]::TerminateExactProcess($Owner.ProcessHandle) }
            catch { $errors.Add("direct_exact_terminate_failed: $($_.Exception.Message)") }
        }
        try { $wait = [CodexDiscordSoak.ProcessOwnershipNative]::WaitExactProcess($Owner.ProcessHandle, [uint32]$TimeoutMilliseconds) }
        catch { $errors.Add("bounded_exact_wait_failed: $($_.Exception.Message)"); $wait = 258 }
        if ($wait -ne 0) {
            try { [CodexDiscordSoak.ProcessOwnershipNative]::TerminateExactProcess($Owner.ProcessHandle) }
            catch { $errors.Add("direct_exact_terminate_failed: $($_.Exception.Message)") }
            try { $wait = [CodexDiscordSoak.ProcessOwnershipNative]::WaitExactProcess($Owner.ProcessHandle, [uint32]$TimeoutMilliseconds) }
            catch { $errors.Add("final_exact_wait_failed: $($_.Exception.Message)"); $wait = 258 }
        }
    }
    if ($wait -ne 0) {
        $errors.Add('Using retained original Process for the fail-closed exit barrier')
        try { if (-not $Owner.Process.HasExited) { $Owner.Process.Kill() } } catch { $errors.Add($_.Exception.Message) }
        while (-not $Owner.ExitConfirmed) {
            try { $Owner.Process.WaitForExit(); $Owner.ExitConfirmed = $true; $wait = 0 }
            catch { if (-not $errors.Contains($_.Exception.Message)) { $errors.Add($_.Exception.Message) }; Start-Sleep -Seconds 1 }
        }
    }
    $Owner.ExitConfirmed = $true
    [pscustomobject]@{ exit_confirmed = $true; pid = $Owner.Pid
        start_ticks = $Owner.StartTicks; cleanup_errors = [object[]]@($errors) }
}

function Close-CodexSoakProcessOwner {
    param([Parameter(Mandatory)][object]$Owner)
    if ($null -ne $Owner.Process -and -not $Owner.ExitConfirmed) { throw 'Refusing to release process ownership before exact child exit confirmation' }
    if ($null -ne $Owner.JobHandle) { $Owner.JobHandle.Dispose(); $Owner.JobHandle = $null }
    if ($null -ne $Owner.ProcessHandle) { $Owner.ProcessHandle.Dispose(); $Owner.ProcessHandle = $null }
    $Owner.Process = $null
}

Export-ModuleMember -Function @(
    'New-CodexSoakProcessOwner', 'Register-CodexSoakOwnedProcess',
    'Stop-CodexSoakOwnedProcess', 'Close-CodexSoakProcessOwner'
)
