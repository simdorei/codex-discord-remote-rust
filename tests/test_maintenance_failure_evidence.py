"""Failure evidence must remain accurate and immutable after a terminal shutdown error."""
import unittest
from test_maintenance_v2_engine import MaintenanceV2EngineTests


class MaintenanceFailureEvidenceTests(unittest.TestCase):
    run_case = MaintenanceV2EngineTests.run_case

    def test_halted_reentry_preserves_original_error_and_observation(self):
        self.run_case(r'''
$script:fail='ACK'
$s=Read-CdrMaintenanceState $StatePath
$s|Add-Member FailureNoticePhase 'none';Save-CdrMaintenanceState $s $StatePath
$script:posts=0;$script:original='alive'
function Get-CdrMaintenanceFailureObservation {[pscustomobject]@{Original=$script:original;Code='original_error'}}
function Publish-CdrMaintenanceFailure {param($s,$p)
 if($s.FailureNoticePhase -ne 'none'){return}
 $script:posts++;$s.FailureNoticePhase='unknown';Save-CdrMaintenanceState $s $p
}
try{Invoke-CdrMaintenanceEngine $StatePath $op}catch{}
$first=Read-CdrMaintenanceState $StatePath
if(-not $first.Halted){throw 'fixture did not halt'}
$errorText=$first.LastError;$observation=$first.FailureObservation|ConvertTo-Json -Compress
$script:original='exited'
try{Invoke-CdrMaintenanceEngine $StatePath $op}catch{}
$second=Read-CdrMaintenanceState $StatePath
if($second.LastError -cne $errorText -or ($second.FailureObservation|ConvertTo-Json -Compress) -cne $observation){throw 'terminal evidence overwritten'}
if($second.FailureNoticePhase -cne 'unknown' -or $script:posts -ne 1){throw 'unknown notice replayed'}
''')

    def test_only_matching_sealed_ack_is_reported_as_matching(self):
        for ack_state in ('sealed', 'open', 'missing', 'foreign'):
            with self.subTest(ack_state=ack_state):
                self.run_case(r'''
. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceFailure.ps1')
. (Join-Path $env:V2_SOURCE 'codex-discord-rust-drain.ps1')
$s=Read-CdrMaintenanceState $StatePath;$s.LastError='fixture_failure'
$DrainAckPath=Join-Path $RepoRoot 'ack';$StopPath=Join-Path $RepoRoot 'stop'
function Get-Process {param($Id,$ErrorAction) $null}
$raw="version=1`nruntime_id=runtime1`nprocess_identity=42|99`nnonce=$op`n"
if('ACKSTATE' -ne 'missing'){$raw+="state=ACKSTATE`n"}
if('ACKSTATE' -eq 'foreign'){$raw=$raw.Replace('state=foreign','state=sealed').Replace($op,('f'*32))}
[IO.File]::WriteAllText($DrainAckPath,$raw)
$observation=Get-CdrMaintenanceFailureObservation $s
if('ACKSTATE' -eq 'sealed'){
 if($observation.Ack -cne 'matching'){throw 'valid ACK not observed'}
}elseif($observation.Ack -ceq 'matching'){throw 'unsealed ACK reported confirmed'}
'''.replace('ACKSTATE',ack_state))

    def test_stop_observation_requires_exact_owner_and_reports_read_failure(self):
        for variant in ('exact','newline','space','unreadable'):
            with self.subTest(variant=variant):
                self.run_case(r'''
. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceFailure.ps1')
$s=Read-CdrMaintenanceState $StatePath;$s.LastError='fixture'
$DrainAckPath=Join-Path $RepoRoot 'ack';$StopPath=Join-Path $RepoRoot 'stop'
function Get-Process {param($Id,$ErrorAction) $null}
$text=$op
if('VARIANT' -eq 'newline'){$text+="`n"}
if('VARIANT' -eq 'space'){$text=' '+$text}
[IO.File]::WriteAllText($StopPath,$text)
$guard=$null
try {
 if('VARIANT' -eq 'unreadable'){$guard=[IO.File]::Open($StopPath,'Open','ReadWrite','None')}
 $observation=Get-CdrMaintenanceFailureObservation $s
 $expected=if('VARIANT' -eq 'exact'){'owned'}elseif('VARIANT' -eq 'unreadable'){'unknown'}else{'foreign'}
 if($observation.Stop -cne $expected){throw 'stop observation differs from owner guard'}
}finally{if($guard){$guard.Dispose()}}
'''.replace('VARIANT',variant))


if __name__ == '__main__':
    unittest.main()
