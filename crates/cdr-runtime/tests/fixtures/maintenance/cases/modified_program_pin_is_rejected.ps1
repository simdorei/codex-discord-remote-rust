param([string]$Variant)
switch ($Variant) {
'0' {
. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceState.ps1')
$s=Read-CdrMaintenanceState $StatePath
$pins=@(Get-CdrMaintenanceProgramPaths|ForEach-Object{
 $path=Join-Path $RepoRoot $_
 [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($path))
 [IO.File]::WriteAllText($path,'fixture code')
 [pscustomobject]@{Path=$_;Hash=(Get-CdrArtifactHash $path)}
})
$s|Add-Member ProgramPins $pins
Assert-CdrMaintenanceProgramPins $s
[IO.File]::WriteAllText((Join-Path $RepoRoot 'scripts/CdrMaintenanceEngine.ps1'),'changed')
try{Assert-CdrMaintenanceProgramPins $s;throw 'changed code accepted'}
catch{if($_.Exception.Message -notmatch 'program_changed_after_arming'){throw}}
}
default { throw "Unknown native fixture variant: $Variant" }
}
