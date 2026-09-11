param([string]$Variant)
$owner=Enter-CdrControl $RepoRoot
try {
 try{$other=Enter-CdrControl $RepoRoot; $other.Dispose(); throw 'lock ignored'}
 catch{if($_.Exception.Message -notmatch 'cdr_control_busy'){throw}}
}finally{$owner.Dispose()}
$next=Enter-CdrControl $RepoRoot; $next.Dispose()
