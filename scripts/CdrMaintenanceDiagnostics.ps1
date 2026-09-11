# Public diagnostics only: redact before bounding, including both console and state.
function Get-CdrMaintenanceDiagnostic([string]$Text, [string]$EnvironmentPath) {
    if (-not $Text) { return '' }
    try {
        $secrets=[Collections.Generic.List[string]]::new()
        $keyPattern='(?i)(TOKEN|SECRET|PASSWORD|PASSWD|API[_-]?KEY|CREDENTIAL|COOKIE)'
        foreach ($entry in [Environment]::GetEnvironmentVariables().GetEnumerator()) {
            if ([string]$entry.Key -match $keyPattern -and [string]$entry.Value) {
                $secrets.Add([string]$entry.Value)
            }
        }
        if ($EnvironmentPath) {
            # An unreadable configured secret source must never expose raw output.
            foreach ($line in [IO.File]::ReadAllLines($EnvironmentPath,[Text.Encoding]::UTF8)) {
                if ($line -match '^\s*(?:export\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(.*?)\s*$') {
                    $key=$Matches[1];$value=$Matches[2]
                    if ($key -match $keyPattern -and $value) {
                        if ($value -match '^(["''])(.*)\1\s*(?:#.*)?$') { $value=$Matches[2] }
                        else { $value=($value -replace '\s+#.*$','').Trim() }
                        if ($value) { $secrets.Add($value) }
                    }
                }
            }
        }
        foreach ($secret in ($secrets | Sort-Object Length -Descending)) {
            $Text=$Text.Replace($secret,'[redacted]')
        }
        $Text=[regex]::Replace($Text,'(?i)\b(Authorization\s*:\s*(?:Bearer|Bot)\s+)\S+','$1[redacted]')
        $Text=[regex]::Replace($Text,'(?i)\b((?:access_token|refresh_token|api[_-]?key|password|secret)\s*[=:]\s*)[^\s,;]+','$1[redacted]')
        $Text=$Text.Trim()
        if ($Text.Length -gt 1024) { return $Text.Substring(0,1024)+' [truncated]' }
        return $Text
    } catch {
        # Do not include the exception: it can contain the input being protected.
        return '[diagnostic withheld: credential redaction failed]'
    }
}
