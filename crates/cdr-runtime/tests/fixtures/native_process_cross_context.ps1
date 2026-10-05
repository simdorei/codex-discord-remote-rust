# Build only. No WMI peer exists while compilation is running.
param([Parameter(Mandatory=$true)][string]$RunDirectory,
      [Parameter(Mandatory=$true)][string]$Nonce, [switch]$InjectBuildStall)
# Lifetime starts before hashing, assembly compilation, or identity queries.
$started=[Diagnostics.Stopwatch]::GetTimestamp()
$expiry=$started+[Diagnostics.Stopwatch]::Frequency*60
$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
Set-StrictMode -Version Latest
if ($InjectBuildStall) { [IO.File]::WriteAllText((Join-Path $RunDirectory 'before-compilation.marker'),'before Add-Type'); [Threading.Thread]::Sleep([Threading.Timeout]::Infinite) }
if ($Nonce -notmatch '^[0-9a-f]{32}$' -or
    -not [IO.Path]::IsPathRooted($RunDirectory) -or
    [IO.Path]::GetFileName($RunDirectory) -cne ('cdr-cross-'+$Nonce) -or
    (([IO.File]::GetAttributes($RunDirectory) -band [IO.FileAttributes]::ReparsePoint) -ne 0)) { throw 'Invalid build directory' }
function Get-BundlePin {
    $paths=@('native_process_cross_context.ps1','native_process_cross_context.cs','native_process_route_matrix.cs')
    $lines=@($paths | ForEach-Object { $_+':'+(Get-FileHash -LiteralPath (Join-Path $PSScriptRoot $_) -Algorithm SHA256).Hash.ToLowerInvariant() })
    $sha=[Security.Cryptography.SHA256]::Create()
    try { return ([BitConverter]::ToString($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes(($lines -join [char]10))))).Replace('-','').ToLowerInvariant() }
    finally { $sha.Dispose() }
}
$pin=Get-BundlePin
$image=Join-Path $RunDirectory 'native_process_cross_context.exe'
Add-Type -Path @((Join-Path $PSScriptRoot 'native_process_route_matrix.cs'),(Join-Path $PSScriptRoot 'native_process_cross_context.cs')) -OutputAssembly $image -OutputType ConsoleApplication -ReferencedAssemblies System.Management.dll,System.Web.Extensions.dll,System.dll,System.Core.dll
if ((Get-BundlePin) -cne $pin -or [Diagnostics.Stopwatch]::GetTimestamp() -ge $expiry) { throw 'Build changed or exceeded its original lifetime' }
[ordered]@{ image=$image; image_pin=(Get-FileHash -LiteralPath $image -Algorithm SHA256).Hash.ToLowerInvariant()
    bundle=$pin; expiry=$expiry.ToString([Globalization.CultureInfo]::InvariantCulture)
    bootstrap_qpc=$started.ToString([Globalization.CultureInfo]::InvariantCulture); frequency=[Diagnostics.Stopwatch]::Frequency
    diagnostic_only=$true; native_gate_pass=$false } | ConvertTo-Json -Compress
