[CmdletBinding()]
param([Parameter(Mandatory=$true)][ValidateSet('duplicate','unconfirmed','expired','request-change','program-change','gate-busy','dispatch-failure','receipt-prewrite','receipt-partial')][string]$Case)
$ErrorActionPreference='Stop'
function Assert($Condition,[string]$Message) { if (-not $Condition) { throw $Message } }
$controller=Join-Path $PSScriptRoot 'Invoke-CdrDesktopRecovery.ps1'
$library=Join-Path $PSScriptRoot 'CdrDesktopRecovery.ps1'
$fixtureRoot=[IO.Path]::GetFullPath((Join-Path ([IO.Path]::GetTempPath()) ('cdr-controller-fixture-'+[guid]::NewGuid().ToString('N'))))
$tempPrefix=[IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\')+'\cdr-controller-fixture-'
Assert ($fixtureRoot.StartsWith($tempPrefix,[StringComparison]::OrdinalIgnoreCase)) 'Unsafe fixture path.'
$gate=$null;$receiptGate=$null
try {
    [void][IO.Directory]::CreateDirectory((Join-Path $fixtureRoot 'scripts'))
    $engine=Join-Path $fixtureRoot 'scripts\CdrDesktopRecovery.ps1'
    [IO.File]::WriteAllText($engine,(". '"+$library.Replace("'","''")+"'"+[Environment]::NewLine+'function Start-Sleep { param([int]$Seconds,[int]$Milliseconds) }'),[Text.UTF8Encoding]::new($false))
    $force=Join-Path $fixtureRoot 'scripts\CdrForceRestart.ps1'
    [IO.File]::WriteAllText($force,'# Isolated mock force library; never starts a host.')
    $tools=Join-Path $fixtureRoot 'scripts\CdrToolsRecovery.ps1'
    $mockTools=@'
function Invoke-CdrToolsRestart($Plan,[string]$ReceiptPath) {
    if ($Plan.Case -ceq 'receipt-prewrite') {
        Save-CdrDesktopRecoveryReceipt $ReceiptPath @{Phase='stopping';Operation=$Plan.Operation;ThreadId=$Plan.ThreadId}
    }
    [IO.File]::AppendAllText((Join-Path $Plan.RepoRoot 'fixture-effects.log'),('dispatch'+[Environment]::NewLine))
    Save-CdrDesktopRecoveryReceipt $ReceiptPath @{Phase='restarting_bot';Operation=$Plan.Operation;ThreadId=$Plan.ThreadId}
    if ($Plan.Case -ceq 'dispatch-failure') { throw 'injected failure after durable partial phase' }
    if ($Plan.Case -ceq 'receipt-partial') {
        # Remains held through the real worker catch; only this fixture child exits it.
        $script:fixtureReceiptLock=[IO.File]::Open($ReceiptPath,'Open','Read',[IO.FileShare]::Read)
    }
    Save-CdrDesktopRecoveryReceipt $ReceiptPath @{Phase='restarted';Operation=$Plan.Operation;ThreadId=$Plan.ThreadId;ToolProbeVerified=$false}
}
'@
    [IO.File]::WriteAllText($tools,$mockTools,[Text.UTF8Encoding]::new($false))
    $relativePins=@('scripts/CdrToolsRecovery.ps1','codex-discord-rust-restart.ps1','codex-discord-rust-watchdog.ps1','codex-discord-rust-control.ps1','codex-discord-rust-drain.ps1','scripts/CdrLaunchJournal.ps1')
    foreach ($relative in $relativePins | Select-Object -Skip 1) {
        [IO.File]::WriteAllText((Join-Path $fixtureRoot $relative),'# Isolated inert program pin.')
    }
    $operation='1234567890abcdef1234567890abcdef'
    $base=Join-Path $fixtureRoot 'maintenance_backups\desktop-recovery'
    $bundle=Join-Path $base $operation
    [void][IO.Directory]::CreateDirectory($bundle)
    $request=Join-Path $bundle 'request.json'
    $receipt=Join-Path $bundle 'receipt.json'
    $attempted=Join-Path $bundle 'attempted'
    $effects=Join-Path $fixtureRoot 'fixture-effects.log'
    $expiry=[DateTime]::UtcNow.AddMinutes(2)
    if ($Case -ceq 'expired') { $expiry=[DateTime]::UtcNow.AddMinutes(-1) }
    $payload=@{
        Version=1;Operation=$operation;RepoRoot=$fixtureRoot;ToolRecovery=$true;ExpiresAt=$expiry.ToString('o')
        Plan=@{RepoRoot=$fixtureRoot;ThreadId='aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee';Operation=$operation;Case=$Case}
        EngineHash=(Get-FileHash -LiteralPath $engine -Algorithm SHA256).Hash
        ForceHash=(Get-FileHash -LiteralPath $force -Algorithm SHA256).Hash
        ControllerHash=(Get-FileHash -LiteralPath $controller -Algorithm SHA256).Hash
        ProgramPins=@($relativePins | ForEach-Object { @{Path=$_;Hash=(Get-FileHash -LiteralPath (Join-Path $fixtureRoot $_) -Algorithm SHA256).Hash} })
    }
    [IO.File]::WriteAllText($request,($payload | ConvertTo-Json -Depth 8),[Text.UTF8Encoding]::new($false))
    $requestHash=(Get-FileHash -LiteralPath $request -Algorithm SHA256).Hash
    [IO.File]::WriteAllText($receipt,(@{Phase='dispatched';Operation=$operation;Evidence='preserve-original'} | ConvertTo-Json))
    if ($Case -ceq 'unconfirmed') {
        [IO.File]::WriteAllText($attempted,'')
        [IO.File]::WriteAllText($receipt,(@{Phase='restarting_bot';Operation=$operation;Evidence='accepted-but-unconfirmed'} | ConvertTo-Json))
    }
    if ($Case -ceq 'request-change') { [IO.File]::AppendAllText($request,([Environment]::NewLine+' ')) }
    if ($Case -ceq 'program-change') { [IO.File]::AppendAllText($tools,([Environment]::NewLine+'# changed after pin')) }
    if ($Case -ceq 'gate-busy') { $gate=[IO.File]::Open((Join-Path $base 'desktop.lock'),'OpenOrCreate','ReadWrite','None') }
    if ($Case -ceq 'receipt-prewrite') { $receiptGate=[IO.File]::Open($receipt,'Open','Read',[IO.FileShare]::Read) }
    $before=(Get-FileHash -LiteralPath $receipt -Algorithm SHA256).Hash
    function Run-Worker {
        $oldPreference=$ErrorActionPreference
        try {
            $ErrorActionPreference='Continue'
            $script:workerOutput=& powershell.exe -NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File $controller -Mode Worker -RepoRoot $fixtureRoot -Operation $operation -RequestHash $requestHash 2>&1
            $script:workerExit=$LASTEXITCODE
        } finally { $ErrorActionPreference=$oldPreference }
    }
    Run-Worker
    $expectedExit=if($Case -ceq 'duplicate'){0}else{1}
    Assert ($script:workerExit -eq $expectedExit) ("Unexpected first worker exit: "+$script:workerExit+" "+($script:workerOutput -join [Environment]::NewLine))
    $dispatchCount=if([IO.File]::Exists($effects)){[IO.File]::ReadAllLines($effects).Length}else{0}
    $expectedDispatch=if($Case -in @('duplicate','dispatch-failure','receipt-partial')){1}else{0}
    Assert ($dispatchCount -eq $expectedDispatch) 'Unexpected host-dispatch count.'
    $result=[IO.File]::ReadAllText($receipt) | ConvertFrom-Json
    if ($Case -in @('expired','request-change','program-change','unconfirmed')) {
        Assert ((Get-FileHash -LiteralPath $receipt -Algorithm SHA256).Hash -ceq $before) 'Pre-claim rejection rewrote existing evidence.'
        Assert ([IO.File]::Exists($attempted) -eq ($Case -ceq 'unconfirmed')) 'Unexpected operation claim.'
    } else {
        Assert ([IO.File]::Exists($attempted)) 'Dispatched/failed operation lost its one-use claim.'
        Assert ($result.Operation -ceq $operation) 'Operation identity was lost.'
        $expectedPhase=switch ($Case) { 'duplicate' {'restarted'} 'receipt-prewrite' {'dispatched'} 'receipt-partial' {'restarting_bot'} default {'failed'} }
        Assert ($result.Phase -ceq $expectedPhase) 'Incorrect durable terminal phase.'
        if ($Case -in @('receipt-prewrite','receipt-partial')) {
            Assert (($script:workerOutput -join [Environment]::NewLine) -match 'Move-Item|MoveFileInfoItem|IOException') 'Receipt storage failure was not exposed by the worker.'
        }
        if ($Case -ceq 'receipt-prewrite') {
            Assert ((Get-FileHash -LiteralPath $receipt -Algorithm SHA256).Hash -ceq $before) 'Failed pre-effect receipt write changed original evidence.'
        }
        if ($Case -ceq 'duplicate') { Assert (-not $result.ToolProbeVerified) 'Host restart was reported as a successful tool probe.' }
        if ($null -ne $gate) { $gate.Dispose();$gate=$null }
        if ($null -ne $receiptGate) { $receiptGate.Dispose();$receiptGate=$null }
        $committed=(Get-FileHash -LiteralPath $receipt -Algorithm SHA256).Hash
        Run-Worker
        Assert ($script:workerExit -eq 1) 'The same operation was accepted twice.'
        Assert ((Get-FileHash -LiteralPath $receipt -Algorithm SHA256).Hash -ceq $committed) 'Duplicate worker replaced prior outcome/evidence.'
        $afterCount=if([IO.File]::Exists($effects)){[IO.File]::ReadAllLines($effects).Length}else{0}
        Assert ($afterCount -eq $expectedDispatch) 'Duplicate operation dispatched a host effect again.'
    }
    Write-Output "recovery_controller_case_passed:$Case"
} finally {
    if ($null -ne $gate) { $gate.Dispose() }
    if ($null -ne $receiptGate) { $receiptGate.Dispose() }
    if ($fixtureRoot.StartsWith($tempPrefix,[StringComparison]::OrdinalIgnoreCase) -and [IO.Directory]::Exists($fixtureRoot)) {
        Remove-Item -LiteralPath $fixtureRoot -Recurse -Force
    }
}
