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
class WatchdogDesktopTests(unittest.TestCase):
    def run_desktop_probe(self, *, running: bool) -> subprocess.CompletedProcess[str]:
        running_literal = "$true" if running else "$false"
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
            $names = @('Test-ChatGptDesktopProcessAlive', 'Ensure-ChatGptDesktopRunning')
            foreach ($name in $names) {{
                $functionAst = $ast.Find({{
                    param($node)
                    $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and
                        $node.Name -eq $name
                }}, $true)
                if ($null -eq $functionAst) {{
                    throw "watchdog desktop function missing: $name"
                }}
                Invoke-Expression $functionAst.Extent.Text
            }}

            $script:Running = {running_literal}
            $script:PackageCalls = 0
            $script:LaunchCalls = 0
            $script:LaunchFile = ''
            $script:LaunchArgument = ''
            function Get-Process {{
                [CmdletBinding()]
                param([string[]]$Name)
                if ($script:Running) {{
                    return [pscustomobject]@{{
                        Path = 'C:\\Program Files\\WindowsApps\\OpenAI.Codex_1.0_x64__family\\app\\ChatGPT.exe'
                    }}
                }}
                return $null
            }}
            function Get-AppxPackage {{
                [CmdletBinding()]
                param([string]$Name)
                $script:PackageCalls += 1
                return [pscustomobject]@{{ PackageFamilyName = 'OpenAI.Codex_family' }}
            }}
            function Get-AppxPackageManifest {{
                [CmdletBinding()]
                param($Package)
                return [pscustomobject]@{{
                    Package = [pscustomobject]@{{
                        Applications = [pscustomobject]@{{
                            Application = @([pscustomobject]@{{ Id = 'App' }})
                        }}
                    }}
                }}
            }}
            function Start-Process {{
                [CmdletBinding()]
                param([string]$FilePath, [object[]]$ArgumentList)
                $script:LaunchCalls += 1
                $script:LaunchFile = $FilePath
                $script:LaunchArgument = [string]$ArgumentList[0]
            }}
            function Write-LauncherLog {{ param([string]$Message) }}

            Ensure-ChatGptDesktopRunning
            Write-Output "package_calls=$script:PackageCalls"
            Write-Output "launch_calls=$script:LaunchCalls"
            Write-Output "launch_file=$script:LaunchFile"
            Write-Output "launch_argument=$script:LaunchArgument"
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

    def test_missing_chatgpt_desktop_is_started_through_registered_app_id(self) -> None:
        # Given
        running = False

        # When
        completed = self.run_desktop_probe(running=running)

        # Then
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertIn("package_calls=1", completed.stdout)
        self.assertIn("launch_calls=1", completed.stdout)
        self.assertIn("launch_file=explorer.exe", completed.stdout)
        self.assertIn(
            r"launch_argument=shell:AppsFolder\OpenAI.Codex_family!App",
            completed.stdout,
        )

    def test_running_chatgpt_desktop_is_not_started_twice(self) -> None:
        # Given
        running = True

        # When
        completed = self.run_desktop_probe(running=running)

        # Then
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertIn("package_calls=0", completed.stdout)
        self.assertIn("launch_calls=0", completed.stdout)
