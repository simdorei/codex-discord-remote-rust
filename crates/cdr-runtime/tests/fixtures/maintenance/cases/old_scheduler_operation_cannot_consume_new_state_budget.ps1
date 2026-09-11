param([string]$Variant)
$before=[IO.File]::ReadAllBytes($StatePath)
try{Invoke-CdrMaintenanceEngine -StatePath $StatePath -ExpectedOperation ('f'*32);throw 'old scheduler adopted new operation'}
catch{if($_.Exception.Message -notmatch 'expected_operation_mismatch'){throw}}
if([Convert]::ToBase64String($before) -cne [Convert]::ToBase64String([IO.File]::ReadAllBytes($StatePath))){throw 'new state changed'}
if($script:calls.Count){throw 'old operation caused action'}
