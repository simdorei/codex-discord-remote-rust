# Isolated diagnostic. Own one named ETW session; never stop or alter another logger.
param([ValidateSet('capture','contracts','regressions')][string]$Mode='capture')
$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
Set-StrictMode -Version Latest
$clock=[Diagnostics.Stopwatch]::StartNew()
if($Mode -eq 'regressions') {
    & (Join-Path $PSScriptRoot 'native_process_kernel_revision26.ps1')
    exit $LASTEXITCODE
}
Add-Type -Path (Join-Path $PSScriptRoot 'native_process_kernel_trace.cs') -ReferencedAssemblies System.dll,System.Core.dll
if($Mode -eq 'contracts') {
    $cases=[CdrKernelContracts]::Run()
    @{diagnostic_only=$true;native_gate_pass=$false;contracts=$cases}|ConvertTo-Json -Depth 5 -Compress
    if(@($cases|Where-Object {-not $_.pass}).Count -gt 0){exit 1}
    exit 0
}
function Select-KernelSubject {
    param($Rows,$Identity,[long]$ExitBound)
    $starts=@($Rows|Where-Object {$_.Kind -eq 'start' -and [CdrKernelRules]::Identity($_,[uint32]$Identity.process_id,[uint32]$Identity.creator_pid,$Identity.image,[long]$Identity.start_before_qpc,[long]$Identity.start_after_qpc,$ExitBound)})
    $stops=@($Rows|Where-Object {$_.Kind -eq 'stop' -and [CdrKernelRules]::Identity($_,[uint32]$Identity.process_id,[uint32]$Identity.creator_pid,$Identity.image,[long]$Identity.start_before_qpc,[long]$Identity.start_after_qpc,$ExitBound)})
    $exact=$starts.Count -eq 1 -and $stops.Count -eq 1
    if($exact){$exact=[long]$starts[0].EventQpc -le [long]$stops[0].EventQpc}
    @{identity=$Identity;unambiguous_lifecycle=$exact;starts=$starts;stops=$stops;all_pid_rows=@($Rows|Where-Object {$_.ProcessId -eq $Identity.process_id})}
}

function Test-KernelFlag {
    param($Object,[string]$Name,[bool]$Expected)
    if($null -eq $Object){return $false}
    $property=$Object.PSObject.Properties[$Name]
    return $null -ne $property -and $property.Value -is [bool] -and $property.Value -eq $Expected
}
function Test-KernelHelperCleanup {
    param($Inner,$Identity)
    if(-not $Identity.started){return $true}
    if(-not (Test-KernelFlag $Inner 'safe_for_follow_up' $true) -or
       -not (Test-KernelFlag $Inner 'diagnostic_only' $true) -or
       -not (Test-KernelFlag $Inner 'native_gate_pass' $false) -or
       -not (Test-KernelFlag $Inner 'threads_joined' $true)){return $false}
    $owner=$Inner.PSObject.Properties['observer_pid']
    if($null -eq $owner -or ($owner.Value -isnot [int] -and $owner.Value -isnot [long]) -or
       $owner.Value -ne $Identity.process_id){return $false}
    foreach($name in @('cleanup_errors','direct_cleanup_errors','semisync_cleanup_errors')) {
        $value=$Inner.PSObject.Properties[$name]
        if($null -eq $value -or $value.Value -isnot [Array] -or $value.Value.Count -ne 0){return $false}
    }
    $probes=$Inner.PSObject.Properties['probes']
    if($null -eq $probes -or $probes.Value -isnot [Array]){return $false}
    foreach($probe in $probes.Value) {
        if(Test-KernelFlag $probe 'started' $false){continue}
        if(-not (Test-KernelFlag $probe 'started' $true) -or
           -not (Test-KernelFlag $probe 'exit_confirmed' $true) -or
           -not (Test-KernelFlag $probe 'termination_requested' $false)){return $false}
        $parent=$probe.PSObject.Properties['creator_pid']
        $child=$probe.PSObject.Properties['process_id']
        if($null -eq $parent -or ($parent.Value -isnot [int] -and $parent.Value -isnot [long]) -or
           $parent.Value -ne $Identity.process_id -or $null -eq $child -or
           ($child.Value -isnot [int] -and $child.Value -isnot [long]) -or $child.Value -le 0){return $false}
    }
    return $true
}
function Complete-KernelEvidence {
    param($Tap,$Identity,$Stdout,$Stderr,$Cleanup,$Primary)
    $rows=@();$publicRows=@();$inner=$null;$control=$null;$subjects=@();$wmiWindows=@{}
    # This snapshot is independent of helper JSON. It is provisional if the consumer did not join.
    try {
        $rows=@($Tap.Rows.ToArray())
        $publicRows=@(foreach($row in $rows) {
            @{kind=$row.Kind;process_id=$row.ProcessId;parent_process_id=$row.ParentProcessId;image=$row.Image
              version=$row.Version;opcode=$row.Opcode;header_flags=$row.HeaderFlags;event_qpc=$row.EventQpc
              received_qpc=$row.ReceivedQpc;window='unknown'}
        })
        if($Identity.exit_confirmed){$control=Select-KernelSubject $rows $Identity ([long]$Identity.exit_observed_qpc)}
    } catch {
        if($null -eq $Primary){$Primary=$_.Exception.Message}else{$Cleanup.Add(@{stage='etw-snapshot';error=$_.Exception.Message})}
    }
    try {
        if($null -ne $Stdout -and $Stdout.Joined -and -not $Stdout.Overflow){$inner=$Stdout.Text|ConvertFrom-Json}
    } catch {
        if($null -eq $Primary){$Primary=$_.Exception.Message}else{$Cleanup.Add(@{stage='helper-json';error=$_.Exception.Message})}
    }
    try {
        if($null -ne $Stderr -and $Stderr.Joined -and -not [string]::IsNullOrWhiteSpace($Stderr.Text)){throw ('WMI helper stderr: '+$Stderr.Text)}
    } catch {
        if($null -eq $Primary){$Primary=$_.Exception.Message}else{$Cleanup.Add(@{stage='helper-stderr';error=$_.Exception.Message})}
    }
    if($null -ne $inner) {
        try {
            [long]$begin=0;[long]$frequency=0;[long]$end=0
            if(-not [long]::TryParse([string]$inner.window_started_qpc,[ref]$begin) -or $begin -le 0 -or
               -not [long]::TryParse([string]$inner.qpc_frequency,[ref]$frequency) -or $frequency -le 0 -or
               -not [long]::TryParse([string]$inner.window_ended_qpc,[ref]$end) -or $end -lt $begin){throw 'Invalid helper observation window'}
            foreach($row in $publicRows){$row.window=[CdrKernelRules]::Window([long]$row.received_qpc,$begin,$frequency)}
            foreach($probe in $inner.probes) {
                $subject=@{process_id=$probe.process_id;creator_pid=$probe.creator_pid;image='cmd.exe';start_before_qpc=$probe.start_before_qpc;start_after_qpc=$probe.start_after_qpc;created_filetime_utc=$probe.created_filetime_utc;exit_filetime_utc=$probe.exit_filetime_utc}
                $subjects+=Select-KernelSubject $rows $subject $end
            }
            foreach($route in @('queue_rows','direct_rows','semisync_rows')) {
                $all=@($inner.$route)+@($inner.('late_'+$route))
                $coverage=if($route -eq 'queue_rows'){'incomplete-not-absence-evidence'}else{'captured-until-observer-disposal'}
                $wmiWindows[$route]=@{original=@($all|Where-Object {$_.ReceivedMs -ge 0 -and $_.ReceivedMs -lt 5000});tail=@($all|Where-Object {$_.ReceivedMs -ge 5000 -and $_.ReceivedMs -lt 20000});cleanup=@($all|Where-Object {$_.ReceivedMs -ge 20000});cleanup_collection=$coverage}
            }
        } catch {
            if($null -eq $Primary){$Primary=$_.Exception.Message}else{$Cleanup.Add(@{stage='evidence-finalization';error=$_.Exception.Message})}
        }
    }
    $safe=$Tap.Safe -and $Cleanup.Count -eq 0 -and (-not $Identity.started -or ($Identity.exit_confirmed -and -not $Identity.termination_requested))
    if(-not (Test-KernelHelperCleanup $inner $Identity)){$safe=$false}
    $valid=$safe -and $null -eq $Primary -and $Tap.CompleteCapture -and $Identity.exit_code -eq 0 -and
        (Test-KernelFlag $inner 'comparison_valid' $true) -and $null -ne $control -and $control.unambiguous_lifecycle
    return @{rows=$publicRows;inner=$inner;control=$control;subjects=$subjects;wmi_windows=$wmiWindows;primary=$Primary;safe=$safe;valid=$valid}
}

$api=[CdrKernelApi]::new()
$tap=[CdrKernelTap]::new($api)
$child=[Diagnostics.Process]::new()
$identity=[ordered]@{started=$false;process_id=0;creator_pid=$PID;image='powershell.exe';start_before_qpc=$null;start_after_qpc=$null;created_filetime_utc=$null;exit_filetime_utc=$null;exit_observed_qpc=$null;exit_confirmed=$false;exit_code=$null;termination_requested=$false}
$cleanup=[Collections.Generic.List[object]]::new()
$stdout=$null;$stderr=$null;$primary=$null;$inner=$null;$control=$null;$subjects=@();$rows=@();$publicRows=@();$wmiWindows=@{}
try {
    $tap.Start()
    # The existing outer wrapper has a 30s bound. Reserve at least 6s for cleanup.
    if($clock.ElapsedMilliseconds -ge 2000){throw 'Setup deadline exceeded; WMI helper was not started'}
    $info=[Diagnostics.ProcessStartInfo]::new()
    $info.FileName=Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    $info.Arguments='-NoProfile -ExecutionPolicy Bypass -File "'+(Join-Path $PSScriptRoot 'native_process_semisync.ps1')+'" -Mode late-arrival'
    $info.UseShellExecute=$false;$info.CreateNoWindow=$true;$info.WindowStyle=[Diagnostics.ProcessWindowStyle]::Hidden
    $info.RedirectStandardInput=$true;$info.RedirectStandardOutput=$true;$info.RedirectStandardError=$true
    $child.StartInfo=$info
    $identity.start_before_qpc=[Diagnostics.Stopwatch]::GetTimestamp().ToString()
    if($clock.ElapsedMilliseconds -ge 2000){throw 'Helper dispatch deadline exceeded; helper was not started'}
    if(-not $child.Start()){throw 'WMI helper did not start'}
    $identity.started=$true;$identity.process_id=$child.Id
    $identity.start_after_qpc=[Diagnostics.Stopwatch]::GetTimestamp().ToString()
    $identity.created_filetime_utc=$child.StartTime.ToUniversalTime().ToFileTimeUtc().ToString()
    $stdout=[CdrKernelText]::new($child.StandardOutput)
    $stderr=[CdrKernelText]::new($child.StandardError)
    $child.StandardInput.Close()
    $remaining=[Math]::Max(0,24000-[int]$clock.ElapsedMilliseconds)
    if(-not $child.WaitForExit($remaining)){throw 'WMI helper deadline exceeded; diagnostic is inconclusive'}
} catch {$primary=$_.Exception.Message}
finally {
    if($identity.started) {
        $exited=$false
        try {$exited=$child.WaitForExit(0)}catch{$cleanup.Add(@{stage='initial-exit-check';error=$_.Exception.Message})}
        if(-not $exited) {
            $identity.termination_requested=$true
            try {$child.Kill()}catch{$cleanup.Add(@{stage='owned-child-kill';error=$_.Exception.Message})}
        }
        try {
            if(-not $child.WaitForExit(1000)){throw 'WMI helper exit remains unconfirmed'}
            $identity.exit_confirmed=$true
            $identity.exit_observed_qpc=[Diagnostics.Stopwatch]::GetTimestamp().ToString()
            $identity.exit_code=$child.ExitCode
            $identity.exit_filetime_utc=$child.ExitTime.ToUniversalTime().ToFileTimeUtc().ToString()
        }catch{$cleanup.Add(@{stage='child-exit-identity';error=$_.Exception.Message})}
    }
    # Session ownership and cleanup do not depend on successful child output or parsing.
    try {$tap.Complete()}catch{$cleanup.Add(@{stage='etw-complete';error=$_.Exception.Message})}
    foreach($entry in @(@{name='stdout';value=$stdout},@{name='stderr';value=$stderr})) {
        if($null -eq $entry.value){continue}
        try {
            $entry.value.Complete()
            if(-not $entry.value.Joined){throw 'Output reader exit remains unconfirmed'}
            if($entry.value.Overflow){throw 'Output exceeded 2 Mi characters'}
            if($null -ne $entry.value.Error){throw $entry.value.Error}
        }catch{$cleanup.Add(@{stage=$entry.name;error=$_.Exception.Message})}
    }
    try {$child.Dispose()}catch{$cleanup.Add(@{stage='child-dispose';error=$_.Exception.Message})}
}
$evidence=Complete-KernelEvidence $tap $identity $stdout $stderr $cleanup $primary
$safe=$evidence.safe;$valid=$evidence.valid
[ordered]@{
    diagnostic_only=$true;native_gate_pass=$false;comparison_valid=$valid;safe_for_follow_up=$safe
    scope='Owned kernel ETW versus WMI, same two cmd lifecycles; not native QA PASS'
    session_name=[CdrKernelNative]::SessionName;session_guid=$api.SessionGuid.ToString();observer_pid=$PID
    acquisition_budget_ms=5000;late_observation_budget_ms=15000;outer_budget_ms=30000;helper_deadline_ms=24000;elapsed_with_cleanup_ms=$clock.ElapsedMilliseconds
    error=$evidence.primary;cleanup_errors=$cleanup.ToArray();helper=$identity;helper_control=$evidence.control;subjects=$evidence.subjects
    etw=@{start_status=$tap.StartStatus;session_owned=$tap.SessionOwned;consumer_opened=$tap.ConsumerOpened;stop_status=$tap.StopStatus;close_status=$tap.CloseStatus;process_trace_status=$tap.ProcessStatus;consumer_joined=$tap.Joined;api_disposed=$tap.ApiDisposed;statistics=$tap.Stats;raw_count=$tap.RawCount;process_count=$tap.ProcessCount;lifecycle_count=$tap.LifecycleCount;rundown_or_other_opcode_count=$tap.RundownCount;dropped=$tap.Dropped;decoder_error_count=$tap.DecodeErrors;schema_overflow_count=$tap.SchemaOverflow;decoder_failure_count=$tap.DecodeFailures;decoder_errors=$tap.Faults.ToArray();cleanup_errors=$tap.CleanupErrors.ToArray();schemas=$tap.Schemas;snapshot_final=$tap.Joined;rows=$evidence.rows}
    wmi_windows=$evidence.wmi_windows;wmi_evidence=$evidence.inner
}|ConvertTo-Json -Depth 12 -Compress
if($valid){exit 0}else{exit 1}
