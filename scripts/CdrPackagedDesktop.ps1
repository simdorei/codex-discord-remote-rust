# Activate the registered Store app, then verify the returned process identity.
# Launching its executable directly can create an unpackaged process instead.
function Initialize-CdrPackagedDesktopInterop {
    if ('Cdr.PackagedDesktop' -as [type]) { return }
    Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;
namespace Cdr {
    [ComImport, Guid("2e941141-7f97-4756-ba1d-9decde894a3d"),
     InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    internal interface IApplicationActivationManager {
        [PreserveSig] int ActivateApplication(
            [MarshalAs(UnmanagedType.LPWStr)] string appUserModelId,
            [MarshalAs(UnmanagedType.LPWStr)] string arguments,
            uint options, out uint processId);
    }
    public static class PackagedDesktop {
        [DllImport("ole32.dll", ExactSpelling=true)]
        private static extern int CoCreateInstance(ref Guid clsid, IntPtr outer,
            uint context, ref Guid iid,
            [MarshalAs(UnmanagedType.Interface)] out IApplicationActivationManager manager);
        [DllImport("kernel32.dll", CharSet=CharSet.Unicode, ExactSpelling=true)]
        private static extern int GetPackageFullName(IntPtr process,
            ref uint length, StringBuilder packageName);
        private static IApplicationActivationManager CreateManager() {
            var clsid = new Guid("45ba127d-10a8-46ea-8ab7-56ea9078943c");
            var iid = new Guid("2e941141-7f97-4756-ba1d-9decde894a3d");
            IApplicationActivationManager manager;
            // The independent launcher may exit before the activated app does.
            Marshal.ThrowExceptionForHR(CoCreateInstance(ref clsid, IntPtr.Zero,
                4 /* CLSCTX_LOCAL_SERVER */, ref iid, out manager));
            return manager;
        }
        public static void CheckActivationAvailable() {
            var manager = CreateManager();
            Marshal.FinalReleaseComObject(manager);
        }
        public static uint Activate(string appUserModelId) {
            var manager = CreateManager();
            try {
                uint processId;
                Marshal.ThrowExceptionForHR(manager.ActivateApplication(appUserModelId,
                    null, 2 /* AO_NOERRORUI */, out processId));
                if (processId == 0) throw new InvalidOperationException("Activation returned no process.");
                return processId;
            } finally { Marshal.FinalReleaseComObject(manager); }
        }
        public static string GetProcessPackageName(IntPtr process) {
            uint length = 0;
            var result = GetPackageFullName(process, ref length, null);
            if (result == 15700 /* APPMODEL_ERROR_NO_PACKAGE */) return null;
            if (result != 122 /* ERROR_INSUFFICIENT_BUFFER */) throw new Win32Exception(result);
            if (length == 0 || length > 4096) throw new InvalidOperationException("Invalid package identity length.");
            var name = new StringBuilder((int)length);
            result = GetPackageFullName(process, ref length, name);
            if (result != 0) throw new Win32Exception(result);
            return name.ToString();
        }
    }
}
'@
}

function Get-CdrPackagedDesktopRegistration {
    $packages=@(Get-AppxPackage -Name OpenAI.Codex)
    if ($packages.Count -ne 1) { throw 'Expected one registered Codex desktop package.' }
    $package=$packages[0]
    $manifest=Get-AppxPackageManifest -Package $package.PackageFullName
    $apps=@($manifest.Package.Applications.Application | Where-Object {
        ([string]$_.Executable).Replace('/','\') -ieq 'app\ChatGPT.exe'
    })
    if ($apps.Count -ne 1 -or [string]::IsNullOrWhiteSpace([string]$apps[0].Id) -or
        [string]::IsNullOrWhiteSpace([string]$package.PackageFamilyName) -or
        [string]::IsNullOrWhiteSpace([string]$package.PackageFullName)) {
        throw 'Registered Codex desktop application identity is missing or ambiguous.'
    }
    $path=Join-Path $package.InstallLocation 'app\ChatGPT.exe'
    if (-not [IO.File]::Exists($path)) { throw 'Installed desktop executable is missing.' }
    return [pscustomobject]@{InstallLocation=$package.InstallLocation;DesktopPath=$path
        DesktopPackageFullName=$package.PackageFullName
        DesktopAppUserModelId=($package.PackageFamilyName+'!'+$apps[0].Id)}
}

function Assert-CdrPackagedDesktopRegistration($Expected) {
    $current=Get-CdrPackagedDesktopRegistration
    foreach ($name in @('DesktopPath','DesktopPackageFullName','DesktopAppUserModelId')) {
        if ([string]::IsNullOrWhiteSpace([string]$Expected.$name) -or
            $Expected.$name -cne $current.$name) { throw "Desktop registration changed: $name" }
    }
}

function Assert-CdrPackagedDesktopActivationAvailable {
    Initialize-CdrPackagedDesktopInterop
    [Cdr.PackagedDesktop]::CheckActivationAvailable()
}

function Invoke-CdrRegisteredDesktopActivation([string]$AppUserModelId) {
    Initialize-CdrPackagedDesktopInterop
    return [Cdr.PackagedDesktop]::Activate($AppUserModelId)
}

function Get-CdrProcessPackageFullName($Process) {
    Initialize-CdrPackagedDesktopInterop
    return [Cdr.PackagedDesktop]::GetProcessPackageName($Process.Handle)
}

function Start-CdrPackagedDesktop($Plan,[TimeSpan]$WaitBudget=[TimeSpan]::FromSeconds(30)) {
    Assert-CdrPackagedDesktopRegistration $Plan
    $startedId=Invoke-CdrRegisteredDesktopActivation $Plan.DesktopAppUserModelId
    $activated=Get-Process -Id $startedId -ErrorAction Stop
    $result=$null
    try {
        # Holding the activation handle also prevents its PID from being reused
        # while checking the direct relaunch child created by Electron startup.
        [void]$activated.Handle
        $activationTime=$activated.StartTime.ToUniversalTime()
        if ((Get-CdrProcessPackageFullName $activated) -cne $Plan.DesktopPackageFullName -or
            (-not $activated.HasExited -and $activated.Path -ine $Plan.DesktopPath)) {
            throw "Activation process $startedId did not match its registered package/executable."
        }
        $deadline=[DateTime]::UtcNow.Add($WaitBudget)
        do {
            $entries=@(Get-CimInstance Win32_Process -Filter "Name='ChatGPT.exe'" | Where-Object {
                $_.ExecutablePath -ieq $Plan.DesktopPath -and
                $_.CommandLine -notmatch '(?:^|\s)--type=' -and
                ($_.ProcessId -eq $startedId -or ($_.ParentProcessId -eq $startedId -and
                    $_.CreationDate.ToUniversalTime() -ge $activationTime))
            })
            foreach ($entry in $entries) {
                $candidate=Get-Process -Id $entry.ProcessId -ErrorAction SilentlyContinue
                if ($null -eq $candidate) { continue }
                try {
                    [void]$candidate.Handle
                    if ($candidate.HasExited) { continue }
                    if ($candidate.Path -ine $Plan.DesktopPath -or
                        [math]::Abs(($candidate.StartTime.ToUniversalTime()-$entry.CreationDate.ToUniversalTime()).Ticks) -gt 10 -or
                        (Get-CdrProcessPackageFullName $candidate) -cne $Plan.DesktopPackageFullName) {
                        throw 'Desktop relaunch process identity changed.'
                    }
                    $backends=@(Get-CimInstance Win32_Process -Filter "ParentProcessId=$($candidate.Id) AND Name='codex.exe'" |
                        Where-Object { $_.CommandLine -match '(?:^|\s)app-server(?:\s|$)' })
                    if ($backends.Count -gt 0) {
                        if ($null -ne $result) { throw 'Multiple activated desktop hosts were observed.' }
                        $result=$candidate
                        $candidate=$null
                    }
                } finally { if ($null -ne $candidate) { $candidate.Dispose() } }
            }
            if ($null -ne $result) { return $result }
            if ([DateTime]::UtcNow -ge $deadline) { break }
            Start-Sleep -Milliseconds 100
        } while ($true)
        throw "Activation $startedId did not produce a verified packaged desktop backend within the wait budget."
    } catch {
        if ($null -ne $result) { $result.Dispose() }
        throw
    } finally {
        $activated.Dispose()
    }
}
