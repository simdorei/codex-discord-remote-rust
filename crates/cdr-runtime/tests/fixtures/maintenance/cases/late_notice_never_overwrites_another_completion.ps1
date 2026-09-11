param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'CONNECTED.ps1'),[Text.Encoding]::UTF8)))
switch ($Variant) {
'0' {
function Invoke-RestMethod {
 $script:posts++
 [IO.File]::WriteAllText(($StatePath+'.completed'),'{"Operation":"newer-ticket"}')
 [pscustomobject]@{id='12345';channel_id=$s.NotifyChannel}
}
Invoke-CdrMaintenanceEngine $StatePath $op
if((Get-Content ($StatePath+'.completed') -Raw|ConvertFrom-Json).Operation -cne 'newer-ticket'){throw 'late result overwrote newer proof'}
if((Read-CdrMaintenanceNotice $s).Receipt -cne '12345'){throw 'own notice receipt missing'}
}
default { throw "Unknown native fixture variant: $Variant" }
}
