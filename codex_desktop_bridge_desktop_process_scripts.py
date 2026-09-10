from __future__ import annotations


def build_stop_codex_app_server_script() -> str:
    return "\n".join(
        (
            "$matches = @(Get-CimInstance Win32_Process | Where-Object {",
            "  $_.Name -ieq 'codex.exe' -and $_.CommandLine -match 'app-server'",
            "})",
            "if ($matches.Count -eq 0) {",
            "  Write-Output 'no_matching_app_servers'",
            "  exit 0",
            "}",
            "foreach ($process in $matches) {",
            "  try {",
            "    Stop-Process -Id $process.ProcessId -Force -ErrorAction Stop",
            '    Write-Output ("stopped PID={0} EXE={1}" -f $process.ProcessId, $process.ExecutablePath)',
            "  } catch {",
            '    Write-Output ("stop_failed PID={0}: {1}" -f $process.ProcessId, $_.Exception.Message)',
            "  }",
            "}",
        )
    )
