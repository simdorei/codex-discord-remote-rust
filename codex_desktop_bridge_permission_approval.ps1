$helperPath = Join-Path (Get-Location) 'codex_desktop_bridge_permission_approval_helpers.ps1'
try {
  $helperScript = [System.IO.File]::ReadAllText($helperPath, [System.Text.Encoding]::UTF8)
  Invoke-Expression $helperScript
} catch {
  Write-Output ("HELPER_LOAD_FAILED " + $_.Exception.Message)
  exit 7
}

$decision = [string]$env:CODEX_APPROVAL_DECISION
$decision = $decision.Trim().ToLowerInvariant()
if (-not $decision) { Write-Output 'NO_DECISION'; exit 2 }

function Get-FallbackElements {
  param($Window, $RootElement)
  if ($Window -eq $RootElement) { return $null }
  return Get-AllElements $RootElement
}

function Find-ApprovalControl {
  param(
    $PrimaryElements,
    $FallbackElements,
    [string[]]$Needles,
    [string[]]$Regexes,
    [string[]]$RejectNeedles,
    [string[]]$RejectRegexes
  )
  $control = $null
  if ($Regexes.Count -gt 0) {
    $control = Find-ElementByRegexes $PrimaryElements $Regexes $RejectRegexes
  }
  if (-not $control -and $Needles.Count -gt 0) {
    $control = Find-ElementByNeedles $PrimaryElements $Needles $RejectNeedles
  }
  if (-not $control -and $FallbackElements) {
    if ($Regexes.Count -gt 0) {
      $control = Find-ElementByRegexes $FallbackElements $Regexes $RejectRegexes
    }
    if (-not $control -and $Needles.Count -gt 0) {
      $control = Find-ElementByNeedles $FallbackElements $Needles $RejectNeedles
    }
  }
  return $control
}

function Find-CancelControl {
  param($PrimaryElements, $FallbackElements)
  return Find-ApprovalControl $PrimaryElements $FallbackElements (Get-SkipApprovalNeedles) @() @() @()
}

function Find-RememberControl {
  param($PrimaryElements, $FallbackElements)
  return Find-ApprovalControl `
    $PrimaryElements `
    $FallbackElements `
    (Get-RememberApprovalNeedles) `
    @('^2[\.\)]\s*') `
    (Get-NoApprovalNeedles) `
    @('^1[\.\)]', '^3[\.\)]')
}

function Find-AcceptControl {
  param($PrimaryElements, $FallbackElements)
  return Find-ApprovalControl `
    $PrimaryElements `
    $FallbackElements `
    (Get-YesApprovalNeedles) `
    @('^1[\.\)]\s*', '^(Yes)$') `
    ((Get-RememberApprovalNeedles) + (Get-NoApprovalNeedles)) `
    @('^2[\.\)]', '^3[\.\)]')
}

function Find-SubmitControl {
  param($PrimaryElements, $FallbackElements)
  return Find-ApprovalControl `
    $PrimaryElements `
    $FallbackElements `
    (Get-SubmitApprovalNeedles) `
    @('^(Submit)$') `
    @() `
    @()
}

function Write-ApprovalControlNotFound {
  param($PrimaryElements, $FallbackElements)
  if ($FallbackElements) {
    $debugSummary = Get-DebugCandidateSummary $FallbackElements
  } else {
    $debugSummary = Get-DebugCandidateSummary $PrimaryElements
  }
  if ($debugSummary) {
    $message = "APPROVAL_CONTROL_NOT_FOUND " + $debugSummary
  } else {
    $message = 'APPROVAL_CONTROL_NOT_FOUND'
  }
  $automationErrors = Get-ApprovalAutomationErrors
  if ($automationErrors) {
    $message = $message + " ERRORS:" + $automationErrors
  }
  Write-Output $message
}

$handle = Get-CodexWindowHandle
if ($handle -eq [IntPtr]::Zero) { Write-Output 'NO_CODEX_WINDOW'; exit 3 }
[void][Native]::ShowWindow($handle, 9)
[void][Native]::SetForegroundWindow($handle)
[void][Native]::BringWindowToTop($handle)
Start-Sleep -Milliseconds 200

$win = Find-CodexAutomationWindow $handle
if (-not $win) { $win = [System.Windows.Automation.AutomationElement]::RootElement }
$rootElement = [System.Windows.Automation.AutomationElement]::RootElement
$all = $null
$allFallback = $null

for ($attempt = 0; $attempt -lt 5; $attempt++) {
  $all = Get-AllElements $win
  $allFallback = Get-FallbackElements $win $rootElement

  if ($decision -eq 'cancel') {
    $cancel = Find-CancelControl $all $allFallback
    if ($cancel -and (Invoke-Element $cancel)) {
      Write-Output 'ACTION=cancel'
      exit 0
    }
    Start-Sleep -Milliseconds 180
    continue
  }

  $isRememberDecision = $decision -eq 'accept-remember'
  if ($isRememberDecision) {
    $remember = Find-RememberControl $all $allFallback
    if (-not ($remember -and (Invoke-Element $remember))) {
      Start-Sleep -Milliseconds 180
      continue
    }
    Start-Sleep -Milliseconds 180
    $all = Get-AllElements $win
  }

  $option = Find-AcceptControl $all $allFallback
  if (-not ($option -and (Invoke-Element $option))) {
    Start-Sleep -Milliseconds 180
    continue
  }

  Start-Sleep -Milliseconds 180
  $all = Get-AllElements $win
  $submit = Find-SubmitControl $all $allFallback
  $acceptActionName = if ($isRememberDecision) { 'ACTION=accept-remember' } else { 'ACTION=accept' }
  if ($submit -and (Invoke-Element $submit)) {
    Write-Output $acceptActionName
    exit 0
  }
  [System.Windows.Forms.SendKeys]::SendWait("{ENTER}")
  Start-Sleep -Milliseconds 180
  Write-Output ($acceptActionName + '-enter')
  exit 0
}

Write-ApprovalControlNotFound $all $allFallback
exit 6
