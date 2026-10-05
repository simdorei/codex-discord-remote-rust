param(
    [string]$TracePath=(Join-Path $PSScriptRoot 'native_process_kernel_trace.cs'),
    [string]$ComparisonPath=(Join-Path $PSScriptRoot 'native_process_kernel_compare.ps1')
)
$ErrorActionPreference='Stop'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
# No native observation is started: fake API ingress and the real finalization AST only.
Add-Type -Path @($TracePath,(Join-Path $PSScriptRoot 'native_process_kernel_revision26.cs')) -ReferencedAssemblies @('System.dll','System.Core.dll')
$checks=[Collections.Generic.List[object]]::new()
foreach($check in [CdrKernelIngressContracts]::Run()){$checks.Add($check)}
$tokens=$null;$parseErrors=$null
$ast=[Management.Automation.Language.Parser]::ParseFile($ComparisonPath,[ref]$tokens,[ref]$parseErrors)
if($parseErrors.Count -ne 0){throw ($parseErrors|Out-String)}
foreach($statement in $ast.EndBlock.Statements){
    if($statement -is [Management.Automation.Language.FunctionDefinitionAst]){. ([scriptblock]::Create($statement.Extent.Text))}
}
$statements=@($ast.EndBlock.Statements);$mainIndex=-1
for($index=0;$index -lt $statements.Count;$index++){
    if($statements[$index] -is [Management.Automation.Language.TryStatementAst] -and $null -ne $statements[$index].Finally){$mainIndex=$index;break}
}
if($mainIndex -lt 0 -or $statements[-1].Extent.Text -notmatch 'exit 0'){throw 'Real comparison finalization boundary was not found'}
$tail=($statements[($mainIndex+1)..($statements.Count-2)]|ForEach-Object {$_.Extent.Text}) -join [Environment]::NewLine
$finalize=[scriptblock]::Create($tail)
function New-EvidenceRow([string]$Kind,[uint32]$Process,[uint32]$Parent,[string]$Image,[long]$Event,[long]$Received){
    $row=[CdrKernelRow]::new();$row.Kind=$Kind;$row.ProcessId=$Process;$row.ParentProcessId=$Parent;$row.Image=$Image
    $row.Version=4;$row.Opcode=$(if($Kind -eq 'start'){1}else{2});$row.HeaderFlags=64
    $row.EventQpc=$Event.ToString();$row.ReceivedQpc=$Received.ToString();return $row
}
function Invoke-EvidenceCase([string]$Case,[scriptblock]$Finalize){
    $queue=[Collections.Concurrent.ConcurrentQueue[CdrKernelRow]]::new()
    foreach($row in @(
        (New-EvidenceRow 'start' 200 100 'powershell.exe' 150 200),
        (New-EvidenceRow 'stop' 200 100 'powershell.exe' 21500 21600),
        (New-EvidenceRow 'start' 41 200 'cmd.exe' 1200 1250),
        (New-EvidenceRow 'stop' 41 200 'cmd.exe' 1300 1350),
        (New-EvidenceRow 'start' 42 200 'cmd.exe' 1400 1450),
        (New-EvidenceRow 'stop' 42 200 'cmd.exe' 3500 3600)
    )){$queue.Enqueue($row)}
    $tap=[pscustomobject]@{Rows=$queue;Safe=$true;CompleteCapture=$true;Joined=$true;StartStatus=0;SessionOwned=$true;ConsumerOpened=$true
        StopStatus=0;CloseStatus=0;ProcessStatus=0;ApiDisposed=$true;Stats=$null;RawCount=6;ProcessCount=6;LifecycleCount=6;RundownCount=0
        Dropped=0;DecodeErrors=0;SchemaOverflow=0;DecodeFailures=0;Faults=[Collections.Concurrent.ConcurrentQueue[string]]::new()
        CleanupErrors=[Collections.Generic.List[string]]::new();Schemas=@{'4/1/64'=3;'4/2/64'=3}}
    $identity=[ordered]@{started=$true;process_id=200;creator_pid=100;image='powershell.exe';start_before_qpc='100';start_after_qpc='200'
        exit_observed_qpc='22000';exit_confirmed=$true;exit_code=0;termination_requested=$false;created_filetime_utc='10';exit_filetime_utc='20'}
    $helper=@{diagnostic_only=$true;native_gate_pass=$false;safe_for_follow_up=$true;comparison_valid=$true;threads_joined=$true;observer_pid=200
        cleanup_errors=@();direct_cleanup_errors=@();semisync_cleanup_errors=@();window_started_qpc='1000';window_ended_qpc='21000';qpc_frequency=1000
        queue_rows=@();direct_rows=@();semisync_rows=@();late_queue_rows=@();late_direct_rows=@();late_semisync_rows=@()
        probes=@(
            @{started=$true;process_id=41;creator_pid=200;start_before_qpc='1150';start_after_qpc='1250';created_filetime_utc='11';exit_filetime_utc='12';exit_confirmed=$true;termination_requested=$false},
            @{started=$true;process_id=42;creator_pid=200;start_before_qpc='1350';start_after_qpc='1450';created_filetime_utc='13';exit_filetime_utc='14';exit_confirmed=$true;termination_requested=$false})}
    $expectedSafe=$true;$expectedValid=$false;$unknown=$false;$textOverride=$null;$errText=''
    switch -Exact ($Case){
        'normal' {$expectedValid=$true}
        'empty_stdout' {$textOverride='';$expectedSafe=$false;$unknown=$true}
        'malformed_json' {$textOverride='{not-json';$expectedSafe=$false;$unknown=$true}
        'stderr_preserves_rows' {$errText='injected helper stderr'}
        'safe_false' {$helper.safe_for_follow_up=$false;$expectedSafe=$false}
        'safe_null' {$helper.safe_for_follow_up=$null;$expectedSafe=$false}
        'safe_string_false' {$helper.safe_for_follow_up='false';$expectedSafe=$false}
        'safe_string_true' {$helper.safe_for_follow_up='true';$expectedSafe=$false}
        'safe_missing' {$helper.Remove('safe_for_follow_up');$expectedSafe=$false}
        'wrong_helper_identity' {$helper.observer_pid=201;$expectedSafe=$false}
        'cleanup_missing' {$helper.Remove('direct_cleanup_errors');$expectedSafe=$false}
        'cleanup_nonempty' {$helper.semisync_cleanup_errors=@('injected cleanup error');$expectedSafe=$false}
        'threads_unjoined' {$helper.threads_joined=$false;$expectedSafe=$false}
        'probe_forced' {$helper.probes[0].termination_requested=$true;$expectedSafe=$false}
        'probe_unconfirmed' {$helper.probes[1].exit_confirmed=$false;$expectedSafe=$false}
        'window_missing' {$helper.Remove('window_started_qpc');$unknown=$true}
        'frequency_zero' {$helper.qpc_frequency=0;$unknown=$true}
        'helper_not_started' {$identity.started=$false;$identity.exit_confirmed=$false;$identity.exit_code=$null;$textOverride='';$unknown=$true}
        'comparison_failed_clean' {$helper.comparison_valid=$false}
        'tap_unsafe' {$tap.Safe=$false;$expectedSafe=$false}
        default {throw ('Unknown regression case: '+$Case)}
    }
    $text=if($null -ne $textOverride){$textOverride}else{$helper|ConvertTo-Json -Depth 10 -Compress}
    $stdout=[pscustomobject]@{Joined=$true;Overflow=$false;Text=$text}
    $stderr=[pscustomobject]@{Joined=$true;Overflow=$false;Text=$errText}
    $api=[pscustomobject]@{SessionGuid=[Guid]::Empty};$clock=[Diagnostics.Stopwatch]::StartNew()
    $cleanup=[Collections.Generic.List[object]]::new();$primary=$null;$inner=$null;$control=$null
    $subjects=@();$rows=@();$publicRows=@();$wmiWindows=@{}
    $json=(. $Finalize)|Out-String
    $value=$json|ConvertFrom-Json -ErrorAction Stop
    $exported=@($value.etw.rows);$original=@($queue.ToArray());$preserved=$exported.Count -eq 6
    if($preserved){
        for($index=0;$index -lt 6;$index++){
            $a=$exported[$index];$b=$original[$index]
            if($a.process_id -ne $b.ProcessId -or $a.parent_process_id -ne $b.ParentProcessId -or $a.image -cne $b.Image -or
               $a.kind -cne $b.Kind -or $a.event_qpc -cne $b.EventQpc -or $a.received_qpc -cne $b.ReceivedQpc -or
               $a.version -ne $b.Version -or $a.opcode -ne $b.Opcode -or $a.header_flags -ne $b.HeaderFlags){$preserved=$false}
            if($unknown -and $a.window -cne 'unknown'){$preserved=$false}
        }
    }
    $pass=$preserved -and $value.safe_for_follow_up -is [bool] -and $value.safe_for_follow_up -eq $expectedSafe -and
        $value.comparison_valid -is [bool] -and $value.comparison_valid -eq $expectedValid -and
        $value.diagnostic_only -eq $true -and $value.native_gate_pass -eq $false
    return @{name=('evidence_'+$Case);pass=$pass;row_count=$exported.Count;safe=$value.safe_for_follow_up;expected_safe=$expectedSafe
        comparison_valid=$value.comparison_valid;expected_valid=$expectedValid;error=$value.error;result=$value}
}
$cases=@('normal','empty_stdout','malformed_json','stderr_preserves_rows','safe_false','safe_null','safe_string_false','safe_string_true','safe_missing','wrong_helper_identity','cleanup_missing','cleanup_nonempty','threads_unjoined','probe_forced','probe_unconfirmed','window_missing','frequency_zero','helper_not_started','comparison_failed_clean','tap_unsafe')
foreach($case in $cases){$checks.Add((Invoke-EvidenceCase $case $finalize))}
$normal=$checks|Where-Object {$_.name -eq 'evidence_normal'}
$checks.Add(@{name='queue_cleanup_not_absence_proof';pass=($normal.result.wmi_windows.queue_rows.cleanup_collection -ceq 'incomplete-not-absence-evidence')})
$failed=@($checks|Where-Object {-not $_.pass})
[ordered]@{diagnostic_only=$true;native_gate_pass=$false;synthetic_only=$true;native_sessions_started=0;contracts=$checks.ToArray();failed=$failed.Count}|ConvertTo-Json -Depth 18 -Compress
if($failed.Count -ne 0){exit 1}
exit 0
