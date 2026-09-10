# Explicit reviewed source/incident bindings; never refresh historical pins.
function Get-CdrReconcileSourcePaths {
    @((Get-CdrMaintenanceProgramPaths) + @('scripts/Register-CdrMaintenance.ps1',
        'scripts/Complete-CdrMaintenance.ps1','scripts/CdrMaintenanceReconcileManifest.ps1',
        'scripts/CdrMaintenanceReconcile.ps1','scripts/CdrMaintenanceObservation.ps1'))
}

function Assert-CdrReconcileFile([string]$Path, [string]$Root) {
    $full=[IO.Path]::GetFullPath($Path);$base=[IO.Path]::GetFullPath($Root)
    if(-not $full.StartsWith($base+'\',[StringComparison]::OrdinalIgnoreCase)){throw 'reconcile_path_outside_root'}
    $cursor=$full
    while($cursor.Length -ge $base.Length){
        if((Get-Item -LiteralPath $cursor -ErrorAction Stop).Attributes -band [IO.FileAttributes]::ReparsePoint){throw 'reconcile_reparse_path_refused'}
        if($cursor -ceq $base){break};$cursor=[IO.Path]::GetDirectoryName($cursor)
    }
    if((Get-Item -LiteralPath $full).PSIsContainer){throw 'reconcile_regular_file_required'}
}

function Assert-CdrReconcileBindings($Manifest, [string]$StatePath, [switch]$AllowCompleted) {
    if($Manifest.Version -ne 1 -or $Manifest.Kind -cne 'completion-only' -or
       $Manifest.Operation -notmatch '^[a-f0-9]{32}$' -or $Manifest.RepoRoot -cne $RepoRoot){throw 'reconcile_manifest_identity_invalid'}
    $bundle=Join-Path $RepoRoot ('.codex-discord-backups/maintenance-v2-'+$Manifest.Operation)
    $originalRoot=Join-Path $bundle 'completion-original'
    $original=Join-Path $originalRoot 'state.json'
    Assert-CdrReconcileFile $original $RepoRoot
    if((Get-CdrArtifactHash $original) -cne $Manifest.OriginalStateHash){throw 'reconcile_original_state_changed'}
    $s=Get-Content -LiteralPath $original -Raw -Encoding UTF8|ConvertFrom-Json
    if($s.Version -ne 2 -or $s.ShutdownPolicy -cne 'live-handshake-v1' -or $s.CompletionPolicy -or
       $s.Operation -cne $Manifest.Operation -or $s.RepoRoot -cne $RepoRoot -or
       $s.Bundle -cne $bundle -or $s.BinaryPath -cne $BinaryPath -or $s.Phase -cne 'notifying' -or
       $s.Halted -ne $true -or $s.ActiveCommand -or $s.Fence.Nonce -cne $s.Operation -or
       $s.CandidateHash -cne $Manifest.CandidateHash){throw 'reconcile_legacy_incident_not_bound'}
    $expected=@(Get-CdrReconcileSourcePaths)
    if(@($Manifest.SourcePins).Count -ne $expected.Count){throw 'reconcile_reviewed_source_set_incomplete'}
    foreach($path in $expected){
        $pin=@($Manifest.SourcePins|Where-Object {$_.Path -ceq $path})
        $full=Join-Path $RepoRoot $path
        Assert-CdrReconcileFile $full $RepoRoot
        if($pin.Count -ne 1 -or (Get-CdrArtifactHash $full) -cne $pin[0].Hash){throw 'reconcile_reviewed_source_changed'}
    }
    if(-not $s.ProgramPins -or @($s.ProgramPins.Path|Select-Object -Unique).Count -ne @($s.ProgramPins).Count){throw 'reconcile_original_source_set_invalid'}
    foreach($pin in $s.ProgramPins){
        if($pin.Path -cnotin $expected){throw 'reconcile_original_source_path_invalid'}
        $copy=Join-Path $originalRoot $pin.Path
        Assert-CdrReconcileFile $copy $RepoRoot
        if((Get-CdrArtifactHash $copy) -cne $pin.Hash){throw 'reconcile_original_source_changed'}
    }
    if([IO.File]::Exists($StatePath)){
        Assert-CdrReconcileFile $StatePath $RepoRoot
        if((Get-CdrArtifactHash $StatePath) -cne $Manifest.OriginalStateHash){throw 'reconcile_active_state_changed'}
    } elseif(-not $AllowCompleted){throw 'reconcile_active_state_missing'}
    foreach($pair in @(@($BinaryPath,$s.CandidateHash),@($s.CandidatePath,$s.CandidateHash),@($s.OperatorPath,$s.OperatorHash),@($EnvPath,$s.EnvHash))){
        Assert-CdrReconcileFile $pair[0] $RepoRoot
        if((Get-CdrArtifactHash $pair[0]) -cne $pair[1]){throw 'reconcile_runtime_artifact_changed'}
    }
    if($s.CandidatePath -cne (Join-Path $bundle 'candidate.exe') -or $s.OperatorPath -cne (Join-Path $bundle 'operator.exe')){throw 'reconcile_artifact_path_invalid'}
    Assert-CdrMaintenancePreStopBackup $s
    Assert-CdrMaintenanceBackupReceipt $s 'PostStopBackup'
    return $s
}
