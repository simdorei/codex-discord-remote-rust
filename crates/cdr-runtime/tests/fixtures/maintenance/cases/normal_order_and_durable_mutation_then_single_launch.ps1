param([string]$Variant)
Invoke-CdrMaintenanceEngine $StatePath $op
$s=Read-CdrMaintenanceState $StatePath
if($s.Phase -ne 'verified' -or $s.DiscordReceipt -ne '12345'){throw 'not verified'}
$expected=@('prepared:ACK','stop_requested:stop','installing:install','mutation_started:cleanup','launch_ready:launch','launched:heartbeats','notifying:notify')
$previous=-1
foreach($entry in $expected){$index=$script:calls.IndexOf($entry);if($index -le $previous){throw "wrong order $entry"};$previous=$index}
Invoke-CdrMaintenanceEngine $StatePath $op
if(@($script:calls|Where-Object{$_ -like '*:launch'}).Count -ne 1){throw 'relaunch on completion'}
