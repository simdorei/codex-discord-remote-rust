import os
import shutil
import subprocess
import textwrap
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
WATCHDOG = ROOT / "codex-discord-watchdog.ps1"


@unittest.skipUnless(os.name == "nt", "PowerShell watchdog tests run on Windows")
@unittest.skipUnless(shutil.which("powershell.exe"), "powershell.exe is required")
class WatchdogHealthTests(unittest.TestCase):
    def run_health_probe(
        self,
        wmi_cpu: float,
        performance_samples: tuple[float, ...],
    ) -> subprocess.CompletedProcess[str]:
        sample_values = ",".join(str(value) for value in performance_samples)
        command = textwrap.dedent(
            f"""
            $ErrorActionPreference = 'Stop'
            $source = Get-Content -LiteralPath '{WATCHDOG}' -Raw
            $tokens = $null
            $parseErrors = $null
            $ast = [System.Management.Automation.Language.Parser]::ParseInput(
                $source,
                [ref]$tokens,
                [ref]$parseErrors
            )
            $functionAst = $ast.Find({{
                param($node)
                $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and
                    $node.Name -eq 'Get-WatchdogSystemHealthIssue'
            }}, $true)
            Invoke-Expression $functionAst.Extent.Text

            $script:CounterCalls = 0
            $script:PerformanceValues = @({sample_values})
            function Get-CimInstance {{
                [CmdletBinding()]
                param([Parameter(Position = 0)][string]$ClassName)
                if ($ClassName -eq 'Win32_Processor') {{
                    return [pscustomobject]@{{ LoadPercentage = {wmi_cpu} }}
                }}
                return [pscustomobject]@{{ FreePhysicalMemory = 4194304 }}
            }}
            function Get-Counter {{
                [CmdletBinding()]
                param(
                    [Parameter(Position = 0)][string[]]$Counter,
                    [int]$SampleInterval,
                    [int]$MaxSamples
                )
                $script:CounterCalls += 1
                foreach ($value in $script:PerformanceValues) {{
                    [pscustomobject]@{{
                        CounterSamples = [pscustomobject]@{{ CookedValue = $value }}
                    }}
                }}
            }}
            function Write-LauncherLog {{ param([string]$Message) }}
            function Get-BotProcessAgeSeconds {{ return 300 }}
            function Get-WatchdogHeartbeatIssue {{
                param(
                    [string]$HeartbeatPath,
                    [int]$MaxAgeSeconds,
                    [int]$StartupGraceSeconds,
                    [double]$ProcessAgeSeconds
                )
                return ''
            }}

            $HealthCpuPercent = 95
            $HealthFreeMemoryMb = 768
            $result = Get-WatchdogSystemHealthIssue
            Write-Output "result=$result"
            Write-Output "counter_calls=$script:CounterCalls"
            """
        )
        return subprocess.run(
            [
                "powershell.exe",
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                command,
            ],
            capture_output=True,
            encoding="utf-8",
            errors="replace",
            timeout=15,
            check=False,
        )

    def test_high_wmi_cpu_is_rejected_when_performance_average_is_low(self) -> None:
        # Given
        wmi_cpu = 100.0
        performance_samples = (10.0, 12.0, 8.0, 11.0, 9.0)

        # When
        completed = self.run_health_probe(wmi_cpu, performance_samples)

        # Then
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertIn("result=", completed.stdout)
        self.assertNotIn("result=cpu_percent", completed.stdout)
        self.assertIn("counter_calls=1", completed.stdout)

    def test_high_wmi_cpu_is_confirmed_by_high_performance_average(self) -> None:
        # Given
        wmi_cpu = 99.0
        performance_samples = (100.0, 100.0, 100.0, 100.0, 100.0)

        # When
        completed = self.run_health_probe(wmi_cpu, performance_samples)

        # Then
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertIn("result=cpu_percent=100 threshold=95", completed.stdout)
        self.assertIn("counter_calls=1", completed.stdout)

    def test_normal_wmi_cpu_skips_performance_counter(self) -> None:
        # Given
        wmi_cpu = 50.0
        performance_samples = (100.0, 100.0, 100.0, 100.0, 100.0)

        # When
        completed = self.run_health_probe(wmi_cpu, performance_samples)

        # Then
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertIn("result=", completed.stdout)
        self.assertNotIn("result=cpu_percent", completed.stdout)
        self.assertIn("counter_calls=0", completed.stdout)
