$global:Seen=@{}
$global:Expected=@{}
$global:FormatCalls=0
$global:Targets=@()
for($i=0;$i -lt 19;$i++) {
    $edition=if($i -lt 17){'2024'}else{'2021'}
    $source=Join-Path $env:CDR_TEST_ROOT ("source with space/target-$i.rs")
    $global:Expected[$source]=$edition
    $global:Targets+=[pscustomobject]@{src_path=$source;edition=$edition}
}
function cargo {
    foreach($required in @('metadata','--offline','--locked','--no-deps')) {
        if($required -notin $args){throw "Missing metadata option $required"}
    }
    $members=@('fixture-package')
    if($env:CDR_FORMAT_VARIANT -eq 'metadata-incomplete'){$members+= 'missing-package'}
    $global:LASTEXITCODE=0
    [pscustomobject]@{
        workspace_members=$members
        packages=@([pscustomobject]@{id='fixture-package';name='fixture';edition='2024';targets=$global:Targets})
    }|ConvertTo-Json -Depth 8
}
function rustfmt {
    $global:FormatCalls++
    if($args[0] -cne '--edition' -or $args[2] -cne '--check'){throw 'Formatter flags changed'}
    $sources=@($args|Select-Object -Skip 3)
    if($sources.Count -lt 1 -or $sources.Count -gt 8){throw 'Unbounded or empty formatting batch'}
    foreach($source in $sources){
        if($global:Expected[$source] -cne $args[1]){throw 'Wrong target edition or changed path'}
        if($global:Seen.ContainsKey($source)){throw 'Duplicate target root'}
        $global:Seen[$source]=$true
    }
    $global:LASTEXITCODE=0
    if($env:CDR_FORMAT_VARIANT -eq 'format-failure' -and $global:FormatCalls -eq 2){$global:LASTEXITCODE=9}
}
$failure=$null
try { & $env:CDR_FORMAT_SCRIPT -RepoRoot $env:CDR_TEST_ROOT }
catch { $failure=$_.Exception.Message }
switch($env:CDR_FORMAT_VARIANT){
    'complete' {
        if($failure){throw $failure}
        if($global:Seen.Count -ne 19 -or $global:FormatCalls -ne 4){throw 'Workspace target coverage is incomplete'}
    }
    'format-failure' {
        if(-not $failure -or $failure -notlike 'Rust formatting check failed*'){throw 'Native formatter failure became success'}
        if($global:FormatCalls -ne 2){throw 'Unexpected failure propagation'}
    }
    'metadata-incomplete' {
        if(-not $failure -or $failure -notlike 'Incomplete Cargo workspace metadata*'){throw 'Missing workspace member became success'}
        if($global:FormatCalls -ne 0){throw 'Formatting started with incomplete metadata'}
    }
    default {throw 'Unknown formatting fixture variant'}
}
exit 0
