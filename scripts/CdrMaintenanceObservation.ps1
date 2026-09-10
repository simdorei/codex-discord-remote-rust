# Pure observations for completion-only recovery: no active-state, DB or process writes.
function Get-CdrCompletionProcessIdentity($Process) {
    $ticks=$Process.StartTime.ToUniversalTime().Ticks;$ticks-=($ticks%10)
    "$([int]$Process.Id)|$ticks"
}

function Get-CdrCompletionObservation($State, [string]$ExpectedChild) {
    $j=Read-CdrLaunchJournal (Join-Path $State.Bundle 'launch.json')
    if(-not $j -or $j.Phase -cne 'child' -or $j.Operation -cne ('maintenance:'+$State.Operation) -or
       $j.ChildIdentity -cne $ExpectedChild -or $j.ArtifactHash -cne $State.CandidateHash -or
       -not (Test-RestartDrainFenceMatch $j.Fence $State.Fence)){throw 'reconcile_launch_journal_mismatch'}
    $all=@(Get-Process -Name cdr-runtime -ErrorAction SilentlyContinue)
    if($all.Count -ne 1){throw 'reconcile_requires_one_runtime'}
    $child=$all[0]
    if((Get-CdrCompletionProcessIdentity $child) -cne $ExpectedChild -or $child.Path -cne $BinaryPath){throw 'reconcile_child_identity_changed'}
    if(Get-Process -Id ([int]$State.Fence.ProcessIdentity.Split('|')[0]) -ErrorAction SilentlyContinue){throw 'reconcile_original_pid_present'}
    $lock=[IO.File]::ReadAllText($LockPath)
    if($lock -notmatch '(?m)^pid=(\d+)\r?$' -or [int]$Matches[1] -ne $child.Id){throw 'reconcile_runtime_lock_changed'}
    $heartbeat=[IO.File]::ReadAllText($HeartbeatPath)
    if($heartbeat -notmatch '(?m)^pid=(\d+)\r?$' -or [int]$Matches[1] -ne $child.Id){throw 'reconcile_heartbeat_owner_changed'}
    if($heartbeat -notmatch '(?m)^updated_at=(\d+)\r?$'){throw 'reconcile_heartbeat_invalid'}
    $stamp=[long]$Matches[1];$now=[DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
    $started=([DateTimeOffset]$child.StartTime.ToUniversalTime()).ToUnixTimeSeconds()
    if($stamp -lt $started -or $stamp -gt $now -or ($now-$stamp) -gt 45){throw 'reconcile_heartbeat_not_fresh'}
    return $stamp
}

function Measure-CdrCompletionProof($State, [string]$ExpectedChild) {
    $first=Get-CdrCompletionObservation $State $ExpectedChild
    $deadline=[DateTimeOffset]::UtcNow.AddSeconds(20)
    do {
        Start-Sleep -Milliseconds 250
        $next=Get-CdrCompletionObservation $State $ExpectedChild
        if($next -gt $first){
            return [pscustomobject]@{Operation=$State.Operation;ChildIdentity=$ExpectedChild;
                CandidateHash=$State.CandidateHash;Heartbeats=@($first,$next);ObservedAt=[DateTimeOffset]::UtcNow.ToString('o')}
        }
    }while([DateTimeOffset]::UtcNow -lt $deadline)
    throw 'reconcile_two_fresh_heartbeats_missing'
}
