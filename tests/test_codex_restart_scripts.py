import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
PLUGIN_RESTART = ROOT / "plugins" / "codex-discord-remote" / "scripts" / "restart.ps1"
PLUGIN_STATUS = ROOT / "plugins" / "codex-discord-remote" / "scripts" / "status.ps1"
WATCHDOG = ROOT / "codex-discord-watchdog.ps1"
ATOMIC_RUNTIME = ROOT / "codex-discord-atomic-file-runtime.ps1"
WATCHDOG_RUNTIME = ROOT / "codex-discord-watchdog-runtime.ps1"
WATCHDOG_RESTART_RUNTIME = ROOT / "codex-discord-watchdog-restart-runtime.ps1"
WATCHDOG_HEARTBEAT_RUNTIME = ROOT / "codex-discord-watchdog-heartbeat-runtime.ps1"
WATCHDOG_IDENTITY_RUNTIME = ROOT / "codex-discord-watchdog-identity-runtime.ps1"
WATCHDOG_STOP_RUNTIME = ROOT / "codex-discord-watchdog-stop-runtime.ps1"
PYTHON_RUNTIME = ROOT / "codex-discord-python-runtime.ps1"


def _write_fake_restart_repo(
    repo_root: Path, bridge_line: str, bridge_stderr: str = ""
) -> None:
    _ = (repo_root / ".codex_discord_runtime").write_text(
        "python\n", encoding="utf-8"
    )
    _ = (repo_root / "codex-discord-watchdog.ps1").write_text(
        WATCHDOG.read_text(encoding="utf-8"),
        encoding="utf-8",
    )
    _ = (repo_root / "codex-discord-watchdog-runtime.ps1").write_text(
        WATCHDOG_RUNTIME.read_text(encoding="utf-8"),
        encoding="utf-8",
    )
    _ = (repo_root / "codex-discord-atomic-file-runtime.ps1").write_text(
        ATOMIC_RUNTIME.read_text(encoding="utf-8"),
        encoding="utf-8",
    )
    _ = (repo_root / "codex-discord-watchdog-restart-runtime.ps1").write_text(
        WATCHDOG_RESTART_RUNTIME.read_text(encoding="utf-8"),
        encoding="utf-8",
    )
    _ = (repo_root / "codex-discord-watchdog-heartbeat-runtime.ps1").write_text(
        WATCHDOG_HEARTBEAT_RUNTIME.read_text(encoding="utf-8"),
        encoding="utf-8",
    )
    _ = (repo_root / "codex-discord-watchdog-identity-runtime.ps1").write_text(
        WATCHDOG_IDENTITY_RUNTIME.read_text(encoding="utf-8"),
        encoding="utf-8",
    )
    _ = (repo_root / "codex-discord-watchdog-stop-runtime.ps1").write_text(
        WATCHDOG_STOP_RUNTIME.read_text(encoding="utf-8"),
        encoding="utf-8",
    )
    if PYTHON_RUNTIME.exists():
        _ = (repo_root / "codex-discord-python-runtime.ps1").write_text(
            PYTHON_RUNTIME.read_text(encoding="utf-8"),
            encoding="utf-8",
        )
    _ = (repo_root / "codex_discord_bot.py").write_text("", encoding="utf-8")
    bridge_script = "import sys\n\n"
    if bridge_stderr:
        bridge_script += f"print({bridge_stderr!r}, file=sys.stderr)\n"
    bridge_script += f"print({bridge_line!r})\n"
    _ = (repo_root / "codex_desktop_bridge.py").write_text(
        bridge_script, encoding="utf-8"
    )


@unittest.skipUnless(os.name == "nt", "PowerShell restart script tests run on Windows")
@unittest.skipUnless(shutil.which("powershell.exe"), "powershell.exe is required")
class RestartScriptTests(unittest.TestCase):
    def run_restart_dry_run(
        self,
        repo_root: Path,
        quiet_seconds: int = 90,
        *,
        use_override: bool = True,
    ) -> subprocess.CompletedProcess[str]:
        env = os.environ.copy()
        if use_override:
            env["CODEX_DISCORD_PYTHON"] = sys.executable
        else:
            _ = env.pop("CODEX_DISCORD_PYTHON", None)
            _ = env.pop("PYTHON_EXE", None)
        return subprocess.run(
            [
                "powershell.exe",
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                str(PLUGIN_RESTART),
                "-RepoRoot",
                str(repo_root),
                "-DryRun",
                "-QuietSeconds",
                str(quiet_seconds),
            ],
            capture_output=True,
            encoding="utf-8",
            errors="replace",
            env=env,
            timeout=30,
            check=False,
        )

    def test_restart_dry_run_allows_old_idle_thread_activity(self) -> None:
        line = (
            "  1 | project:1 | idle | ctx 1/2 | used 1 | rec - | "
            "model gpt-5.5/xhigh/default/fast | thread-id | 2000-01-01 00:00:00 | old"
        )
        with tempfile.TemporaryDirectory() as temp_dir:
            repo_root = Path(temp_dir)
            _write_fake_restart_repo(repo_root, line)

            completed = self.run_restart_dry_run(repo_root)

        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertIn("restart_check_ok", completed.stdout)

    def test_restart_dry_run_rejects_recent_idle_thread_activity(self) -> None:
        line = (
            "  1 | project:1 | idle | ctx 1/2 | used 1 | rec - | "
            "model gpt-5.5/xhigh/default/fast | thread-id | 2099-01-01 00:00:00 | active"
        )
        with tempfile.TemporaryDirectory() as temp_dir:
            repo_root = Path(temp_dir)
            _write_fake_restart_repo(repo_root, line)

            completed = self.run_restart_dry_run(repo_root)

        output = completed.stdout + completed.stderr
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("recent", output)
        self.assertIn("busy or not quiet", output)

    def test_restart_dry_run_allows_bridge_repair_notice_on_stderr(self) -> None:
        line = (
            "  1 | project:1 | idle | ctx 1/2 | used 1 | rec - | "
            "model gpt-5.5/xhigh/default/fast | thread-id | 2000-01-01 00:00:00 | active"
        )
        with tempfile.TemporaryDirectory() as temp_dir:
            repo_root = Path(temp_dir)
            _write_fake_restart_repo(
                repo_root,
                line,
                bridge_stderr="bridge_state_repaired: backup=state.json.corrupt.bak",
            )

            completed = self.run_restart_dry_run(repo_root)

        output = completed.stdout + completed.stderr
        self.assertEqual(completed.returncode, 0, output)
        self.assertIn("restart_check_ok", output)

    def test_restart_dry_run_rejects_busy_thread(self) -> None:
        line = (
            "  1 | project:1 | busy | ctx 1/2 | used 1 | rec - | "
            "model gpt-5.5/xhigh/default/fast | thread-id | 2000-01-01 00:00:00 | active"
        )
        with tempfile.TemporaryDirectory() as temp_dir:
            repo_root = Path(temp_dir)
            _write_fake_restart_repo(repo_root, line)

            completed = self.run_restart_dry_run(repo_root)

        output = completed.stdout + completed.stderr
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("busy", output)
        self.assertIn("Codex threads are busy", output)

    def test_restart_readiness_waits_until_recent_activity_becomes_quiet(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            repo_root = Path(temp_dir)
            _write_fake_restart_repo(repo_root, "")
            counter = repo_root / "bridge-counter"
            bridge_script = "\n".join(
                [
                    "from pathlib import Path",
                    f"counter = Path({str(counter)!r})",
                    "count = int(counter.read_text() or '0') if counter.exists() else 0",
                    "counter.write_text(str(count + 1))",
                    "timestamp = '2099-01-01 00:00:00' if count == 0 else '2000-01-01 00:00:00'",
                    "print('1 | project:1 | idle | ctx 1/2 | used 1 | rec - | model | thread-id | ' + timestamp + ' | active')",
                ]
            )
            _ = (repo_root / "codex_desktop_bridge.py").write_text(
                bridge_script,
                encoding="utf-8",
            )
            env = os.environ.copy()
            env["CODEX_DISCORD_PYTHON"] = sys.executable

            completed = subprocess.run(
                [
                    "powershell.exe",
                    "-NoProfile",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-File",
                    str(repo_root / "codex-discord-watchdog.ps1"),
                    "-CheckRestartReady",
                    "-RestartQuietSeconds",
                    "15",
                    "-RestartWaitTimeoutSeconds",
                    "10",
                ],
                capture_output=True,
                encoding="utf-8",
                errors="replace",
                env=env,
                timeout=20,
                check=False,
            )
            counter_value = int(counter.read_text(encoding="utf-8"))

        output = completed.stdout + completed.stderr
        self.assertEqual(completed.returncode, 0, output)
        self.assertIn("restart_check_ok", output)
        self.assertGreaterEqual(counter_value, 2)

    def test_restart_request_does_not_create_marker_before_final_answer(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            repo_root = Path(temp_dir)
            _write_fake_restart_repo(repo_root, "")
            _ = (repo_root / "codex_discord_bot.py").write_text(
                "import time\ntime.sleep(300)\n",
                encoding="utf-8",
            )
            bot = subprocess.Popen(
                [sys.executable, str(repo_root / "codex_discord_bot.py")],
                creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
            )
            _ = (repo_root / ".codex_discord_bot.runtime.lock").write_text(
                str(bot.pid),
                encoding="ascii",
            )
            command = "\n".join(
                [
                    "function Start-Process {",
                    "    [CmdletBinding()]",
                    "    param([string]$FilePath, [object[]]$ArgumentList, [string]$WindowStyle)",
                    "    Write-Output ('start_process_mock:' + $WindowStyle)",
                    "}",
                    f"& {str(PLUGIN_RESTART)!r} -RepoRoot {str(repo_root)!r} -DelaySeconds 0 -QuietSeconds 0",
                ]
            )
            try:
                completed = subprocess.run(
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
                    timeout=30,
                    check=False,
                )
                marker_exists = (repo_root / ".codex_discord_bot.restart").exists()
            finally:
                bot.kill()
                _ = bot.wait(timeout=3)

        output = completed.stdout + completed.stderr
        self.assertEqual(completed.returncode, 0, output)
        self.assertIn("start_process_mock:Hidden", output)
        self.assertIn("restart_queued: delay_seconds=15 quiet_seconds=15", output)
        self.assertFalse(marker_exists)

    def test_restart_claim_is_bound_to_one_process_incarnation(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            claim_path = Path(temp_dir) / "restart.claimed"
            _ = claim_path.write_text("identity=17|100", encoding="utf-8")
            command = "\n".join(
                [
                    f". {str(WATCHDOG_IDENTITY_RUNTIME)!r}",
                    "function Get-CodexBotProcessIdentity {",
                    "    param([string]$BotScript, [string]$RuntimeLockPath)",
                    "    return '17|200'",
                    "}",
                    "$stale = Test-RestartClaimMatchesCurrentBot "
                    + f"-ClaimPath {str(claim_path)!r} -BotScript 'bot.py' -RuntimeLockPath 'lock'",
                    "[System.IO.File]::WriteAllText("
                    + f"{str(claim_path)!r}, 'identity=17|200')",
                    "$current = Test-RestartClaimMatchesCurrentBot "
                    + f"-ClaimPath {str(claim_path)!r} -BotScript 'bot.py' -RuntimeLockPath 'lock'",
                    "Write-Output ('stale=' + $stale)",
                    "Write-Output ('current=' + $current)",
                ]
            )
            completed = subprocess.run(
                ["powershell.exe", "-NoProfile", "-Command", command],
                capture_output=True,
                encoding="utf-8",
                errors="replace",
                timeout=15,
                check=False,
            )

        output = completed.stdout + completed.stderr
        self.assertEqual(completed.returncode, 0, output)
        self.assertIn("stale=False", completed.stdout)
        self.assertIn("current=True", completed.stdout)

    def test_watchdog_claim_owner_rejects_reused_pid_incarnation(self) -> None:
        command = "\n".join(
            [
                f". {str(WATCHDOG_IDENTITY_RUNTIME)!r}",
                "$owner = [Diagnostics.Process]::GetCurrentProcess()",
                "try {",
                "    $ticks = $owner.StartTime.ToUniversalTime().Ticks",
                "    $ticks -= ($ticks % 10)",
                "} finally { $owner.Dispose() }",
                "$current = Test-WatchdogClaimOwnerAlive "
                + "-ClaimName ('.claimed.' + $PID + '.' + $ticks)",
                "$reused = Test-WatchdogClaimOwnerAlive "
                + "-ClaimName ('.claimed.' + $PID + '.' + ($ticks - 10))",
                "Write-Output ('current=' + $current)",
                "Write-Output ('reused=' + $reused)",
            ]
        )
        completed = subprocess.run(
            ["powershell.exe", "-NoProfile", "-Command", command],
            capture_output=True,
            encoding="utf-8",
            errors="replace",
            timeout=15,
            check=False,
        )

        output = completed.stdout + completed.stderr
        self.assertEqual(completed.returncode, 0, output)
        self.assertIn("current=True", completed.stdout)
        self.assertIn("reused=False", completed.stdout)

    def test_restart_dry_run_uses_python_exe_from_env_file(self) -> None:
        line = (
            "1 | project:1 | idle | ctx 1/2 | used 1 | rec - | "
            "model | thread-id | 2000-01-01 00:00:00 | old"
        )
        with tempfile.TemporaryDirectory() as temp_dir:
            repo_root = Path(temp_dir)
            _write_fake_restart_repo(repo_root, line)
            _ = (repo_root / ".env").write_text(
                f'PYTHON_EXE="{sys.executable}"\n',
                encoding="utf-8",
            )

            completed = self.run_restart_dry_run(
                repo_root,
                use_override=False,
            )

        output = completed.stdout + completed.stderr
        self.assertEqual(completed.returncode, 0, output)
        self.assertIn("restart_check_ok", output)

    def test_status_uses_python_exe_from_env_file(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            repo_root = Path(temp_dir)
            _ = (repo_root / "codex-discord-python-runtime.ps1").write_text(
                PYTHON_RUNTIME.read_text(encoding="utf-8"),
                encoding="utf-8",
            )
            _ = (repo_root / ".env").write_text(
                f'PYTHON_EXE="{sys.executable}"\n',
                encoding="utf-8",
            )
            _ = (repo_root / "codex_desktop_bridge.py").write_text(
                "print('custom-python-bridge-ok')\n",
                encoding="utf-8",
            )
            env = os.environ.copy()
            _ = env.pop("CODEX_DISCORD_PYTHON", None)
            _ = env.pop("PYTHON_EXE", None)
            env["CODEX_DISCORD_RUNTIME"] = "python"

            completed = subprocess.run(
                [
                    "powershell.exe",
                    "-NoProfile",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-File",
                    str(PLUGIN_STATUS),
                    "-RepoRoot",
                    str(repo_root),
                ],
                capture_output=True,
                encoding="utf-8",
                errors="replace",
                env=env,
                timeout=30,
                check=False,
            )

        output = completed.stdout + completed.stderr
        self.assertEqual(completed.returncode, 0, output)
        self.assertIn("custom-python-bridge-ok", output)

    def test_stop_marker_is_complete_when_final_path_becomes_visible(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            target = root / ".codex_discord_bot.stop"
            expected_size = 32 * 1024 * 1024
            command = "\n".join(
                [
                    f"$ScriptDir = {str(root)!r}",
                    f"$LauncherLogPath = {str(root / 'launcher.log')!r}",
                    f"$RuntimeLockPath = {str(root / 'runtime.lock')!r}",
                    f". {str(ATOMIC_RUNTIME)!r}",
                    f". {str(WATCHDOG_RUNTIME)!r}",
                    f"$content = 'x' * {expected_size}",
                    "Publish-AtomicTextFile "
                    + f"-Path {str(target)!r} -Content $content",
                ]
            )
            process = subprocess.Popen(
                ["powershell.exe", "-NoProfile", "-Command", command],
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                encoding="utf-8",
                errors="replace",
            )
            observed_size: int | None = None
            deadline = time.monotonic() + 20
            while time.monotonic() < deadline:
                if target.exists():
                    observed_size = target.stat().st_size
                    break
                if process.poll() is not None:
                    break
                time.sleep(0.0005)
            stdout, stderr = process.communicate(timeout=20)

            self.assertEqual(process.returncode, 0, stdout + stderr)
            self.assertEqual(observed_size, expected_size)
            self.assertEqual(target.stat().st_size, expected_size)

    def test_concurrent_stop_marker_publishers_accept_the_same_request(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            target = root / ".codex_discord_bot.stop"
            gate = root / "go"

            def command_for(ready: Path) -> str:
                return "\n".join(
                    [
                        f"$ScriptDir = {str(root)!r}",
                        f"$LauncherLogPath = {str(root / 'launcher.log')!r}",
                        f"$RuntimeLockPath = {str(root / 'runtime.lock')!r}",
                        f". {str(ATOMIC_RUNTIME)!r}",
                        f". {str(WATCHDOG_RUNTIME)!r}",
                        f"[System.IO.File]::WriteAllText({str(ready)!r}, 'ready')",
                        f"while (-not (Test-Path -LiteralPath {str(gate)!r})) {{",
                        "    Start-Sleep -Milliseconds 5",
                        "}",
                        "Publish-AtomicTextFile "
                        + f"-Path {str(target)!r} -Content 'identity=17|100'",
                    ]
                )

            ready_paths = (root / "ready-1", root / "ready-2")
            processes = [
                subprocess.Popen(
                    [
                        "powershell.exe",
                        "-NoProfile",
                        "-Command",
                        command_for(ready),
                    ],
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    text=True,
                    encoding="utf-8",
                    errors="replace",
                )
                for ready in ready_paths
            ]
            try:
                deadline = time.monotonic() + 15
                while time.monotonic() < deadline and not all(
                    ready.exists() for ready in ready_paths
                ):
                    time.sleep(0.01)
                self.assertTrue(all(ready.exists() for ready in ready_paths))
                _ = gate.write_text("go", encoding="ascii")
                results = [process.communicate(timeout=15) for process in processes]

                for process, (stdout, stderr) in zip(
                    processes,
                    results,
                    strict=True,
                ):
                    self.assertEqual(process.returncode, 0, stdout + stderr)
                self.assertEqual(
                    target.read_text(encoding="utf-8"),
                    "identity=17|100",
                )
            finally:
                for process in processes:
                    if process.poll() is None:
                        process.kill()
                        _ = process.wait(timeout=5)

    def test_stop_marker_publisher_accepts_a_request_consumed_after_collision(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            target = root / ".codex_discord_bot.stop"
            command = "\n".join(
                [
                    f"$ScriptDir = {str(root)!r}",
                    f"$LauncherLogPath = {str(root / 'launcher.log')!r}",
                    f"$RuntimeLockPath = {str(root / 'runtime.lock')!r}",
                    f". {str(ATOMIC_RUNTIME)!r}",
                    f". {str(WATCHDOG_RUNTIME)!r}",
                    "$script:MoveAttempts = 0",
                    "function Move-AtomicTextFile {",
                    "    param([string]$Source, [string]$Destination)",
                    "    $script:MoveAttempts++",
                    "    if ($script:MoveAttempts -eq 1) {",
                    "        [System.IO.File]::WriteAllText("
                    + "$Destination, 'identity=stale')",
                    "        Remove-Item -LiteralPath $Destination -Force",
                    "        throw [System.IO.IOException]::new("
                    + "'target existed', -2147024816)",
                    "    }",
                    "    [System.IO.File]::Move($Source, $Destination)",
                    "}",
                    "Publish-AtomicTextFile "
                    + f"-Path {str(target)!r} -Content 'identity=17|100'",
                ]
            )
            completed = subprocess.run(
                ["powershell.exe", "-NoProfile", "-Command", command],
                capture_output=True,
                encoding="utf-8",
                errors="replace",
                timeout=15,
                check=False,
            )

            output = completed.stdout + completed.stderr
            self.assertEqual(completed.returncode, 0, output)
            self.assertEqual(
                target.read_text(encoding="utf-8"),
                "identity=17|100",
            )
            self.assertEqual(tuple(root.glob(".*.tmp")), ())

    def test_stop_marker_publisher_surfaces_an_unrelated_move_failure(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            target = root / ".codex_discord_bot.stop"
            command = "\n".join(
                [
                    f"$ScriptDir = {str(root)!r}",
                    f"$LauncherLogPath = {str(root / 'launcher.log')!r}",
                    f"$RuntimeLockPath = {str(root / 'runtime.lock')!r}",
                    f". {str(ATOMIC_RUNTIME)!r}",
                    f". {str(WATCHDOG_RUNTIME)!r}",
                    "function Move-AtomicTextFile {",
                    "    throw [System.IO.IOException]::new('disk failure')",
                    "}",
                    "Publish-AtomicTextFile "
                    + f"-Path {str(target)!r} -Content 'identity=17|100'",
                ]
            )
            completed = subprocess.run(
                ["powershell.exe", "-NoProfile", "-Command", command],
                capture_output=True,
                encoding="utf-8",
                errors="replace",
                timeout=15,
                check=False,
            )

            self.assertNotEqual(completed.returncode, 0)
            self.assertFalse(target.exists())
            self.assertEqual(tuple(root.glob(".*.tmp")), ())

    def test_restart_claim_rejects_an_empty_unbound_marker(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            claim_path = Path(temp_dir) / "restart.claimed"
            _ = claim_path.write_text("", encoding="utf-8")
            command = "\n".join(
                [
                    f". {str(WATCHDOG_IDENTITY_RUNTIME)!r}",
                    "function Get-CodexBotProcessIdentity {",
                    "    param([string]$BotScript, [string]$RuntimeLockPath)",
                    "    return '17|200'",
                    "}",
                    "$matches = Test-RestartClaimMatchesCurrentBot "
                    + f"-ClaimPath {str(claim_path)!r} -BotScript 'bot.py' -RuntimeLockPath 'lock'",
                    "Write-Output ('matches=' + $matches)",
                ]
            )
            completed = subprocess.run(
                ["powershell.exe", "-NoProfile", "-Command", command],
                capture_output=True,
                encoding="utf-8",
                errors="replace",
                timeout=15,
                check=False,
            )

        output = completed.stdout + completed.stderr
        self.assertEqual(completed.returncode, 0, output)
        self.assertIn("matches=False", completed.stdout)

    def test_stale_stop_identity_never_stops_the_replacement_bot(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            repo_root = Path(temp_dir)
            bot_script = repo_root / "codex_discord_bot.py"
            runtime_lock = repo_root / ".codex_discord_bot.runtime.lock"
            stop_marker = repo_root / ".codex_discord_bot.stop"
            launcher_log = repo_root / "discord_launcher.log"
            _ = bot_script.write_text(
                "import time\ntime.sleep(300)\n", encoding="utf-8"
            )
            old_bot = subprocess.Popen(
                [sys.executable, str(bot_script)],
                creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
            )
            identity_command = "\n".join(
                [
                    f". {str(WATCHDOG_IDENTITY_RUNTIME)!r}",
                    "$identity = Get-CodexBotProcessIdentityById "
                    + f"-BotScript {str(bot_script)!r} -ProcessId {old_bot.pid}",
                    "Write-Output $identity",
                ]
            )
            new_bot: subprocess.Popen[bytes] | None = None
            try:
                identity_result = subprocess.run(
                    ["powershell.exe", "-NoProfile", "-Command", identity_command],
                    capture_output=True,
                    encoding="utf-8",
                    errors="replace",
                    timeout=15,
                    check=False,
                )
                old_identity = identity_result.stdout.strip()
                self.assertEqual(identity_result.returncode, 0, identity_result.stderr)
                self.assertRegex(old_identity, rf"^{old_bot.pid}\|\d+$")
                old_bot.kill()
                _ = old_bot.wait(timeout=3)

                new_bot = subprocess.Popen(
                    [sys.executable, str(bot_script)],
                    creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
                )
                _ = runtime_lock.write_text(str(new_bot.pid), encoding="ascii")
                stop_command = "\n".join(
                    [
                        f"$ScriptDir = {str(repo_root)!r}",
                        f"$BotScript = {str(bot_script)!r}",
                        f"$RuntimeLockPath = {str(runtime_lock)!r}",
                        f"$StopRequestPath = {str(stop_marker)!r}",
                        f"$LauncherLogPath = {str(launcher_log)!r}",
                        f". {str(WATCHDOG_IDENTITY_RUNTIME)!r}",
                        f". {str(WATCHDOG_RUNTIME)!r}",
                        f"Stop-RuntimeBotProcess -ExpectedIdentity {old_identity!r}",
                    ]
                )
                stopped = subprocess.run(
                    ["powershell.exe", "-NoProfile", "-Command", stop_command],
                    capture_output=True,
                    encoding="utf-8",
                    errors="replace",
                    timeout=15,
                    check=False,
                )

                self.assertEqual(stopped.returncode, 0, stopped.stderr)
                self.assertIsNone(new_bot.poll())
                self.assertFalse(stop_marker.exists())
            finally:
                if old_bot.poll() is None:
                    old_bot.kill()
                    _ = old_bot.wait(timeout=3)
                if new_bot is not None and new_bot.poll() is None:
                    new_bot.kill()
                    _ = new_bot.wait(timeout=3)

    def test_stale_stop_marker_is_removed_before_missing_bot_startup(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            repo_root = Path(temp_dir)
            _write_fake_restart_repo(repo_root, "")
            watchdog_path = repo_root / "codex-discord-watchdog.ps1"
            watchdog_text = watchdog_path.read_text(encoding="utf-8").replace(
                "Ensure-ChatGptDesktopRunning -DryRun:$DryRun",
                "Write-Output 'chatgpt_start_mock'",
            )
            _ = watchdog_path.write_text(watchdog_text, encoding="utf-8")
            _ = (repo_root / "codex-discord-bot-headless.vbs").write_text(
                "",
                encoding="utf-8",
            )
            stop_marker = repo_root / ".codex_discord_bot.stop"
            _ = stop_marker.write_text("identity=999999|1", encoding="utf-8")
            command = "\n".join(
                [
                    "function Start-Process {",
                    "    [CmdletBinding()]",
                    "    param([string]$FilePath, [object[]]$ArgumentList, [string]$WindowStyle)",
                    "    Write-Output ('start_process_mock:' + $FilePath)",
                    "}",
                    f"& {str(watchdog_path)!r}",
                ]
            )

            completed = subprocess.run(
                ["powershell.exe", "-NoProfile", "-Command", command],
                capture_output=True,
                encoding="utf-8",
                errors="replace",
                timeout=20,
                check=False,
            )
            marker_exists = stop_marker.exists()

        output = completed.stdout + completed.stderr
        self.assertEqual(completed.returncode, 0, output)
        self.assertIn("start_process_mock:wscript.exe", output)
        self.assertFalse(marker_exists)

    def test_restart_dry_run_rejects_unknown_activity_timestamp(self) -> None:
        line = "1 | project:1 | idle | ctx 1/2 | used 1 | rec - | model | thread-id | invalid | active"
        with tempfile.TemporaryDirectory() as temp_dir:
            repo_root = Path(temp_dir)
            _write_fake_restart_repo(repo_root, line)

            completed = self.run_restart_dry_run(repo_root)

        output = completed.stdout + completed.stderr
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("unknown_timestamp", output)
        self.assertIn("busy or not quiet", output)

    def test_missing_heartbeat_uses_fixed_process_start_grace(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            heartbeat_path = Path(temp_dir) / "missing.heartbeat"
            command = "\n".join(
                [
                    f". {str(WATCHDOG_HEARTBEAT_RUNTIME)!r}",
                    "$now = [datetime]'2026-08-02T12:00:00'",
                    "$inside = Get-WatchdogHeartbeatIssue -HeartbeatPath "
                    + f"{str(heartbeat_path)!r} -MaxAgeSeconds 45 -StartupGraceSeconds 120 "
                    + "-Now $now -ProcessAgeSeconds 119",
                    "$expired = Get-WatchdogHeartbeatIssue -HeartbeatPath "
                    + f"{str(heartbeat_path)!r} -MaxAgeSeconds 45 -StartupGraceSeconds 120 "
                    + "-Now ($now.AddYears(-1)) -ProcessAgeSeconds 120",
                    "Write-Output ('inside=' + $inside)",
                    "Write-Output ('expired=' + $expired)",
                ]
            )
            completed = subprocess.run(
                ["powershell.exe", "-NoProfile", "-Command", command],
                capture_output=True,
                encoding="utf-8",
                errors="replace",
                timeout=15,
                check=False,
            )

        output = completed.stdout + completed.stderr
        self.assertEqual(completed.returncode, 0, output)
        self.assertIn("inside=", completed.stdout)
        self.assertNotIn("inside=heartbeat_missing", completed.stdout)
        self.assertIn("expired=heartbeat_missing", completed.stdout)
        self.assertIn("process_age_seconds=120", completed.stdout)

    def test_missing_heartbeat_does_not_get_a_second_startup_grace(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            heartbeat_path = Path(temp_dir) / "heartbeat"
            _ = heartbeat_path.write_text("1", encoding="ascii")
            command = "\n".join(
                [
                    f". {str(WATCHDOG_HEARTBEAT_RUNTIME)!r}",
                    "$healthy = Get-WatchdogHeartbeatIssue -HeartbeatPath "
                    + f"{str(heartbeat_path)!r} -MaxAgeSeconds 45 "
                    + "-StartupGraceSeconds 120 -ProcessAgeSeconds 500",
                    f"Remove-Item -LiteralPath {str(heartbeat_path)!r}",
                    "$missing = Get-WatchdogHeartbeatIssue -HeartbeatPath "
                    + f"{str(heartbeat_path)!r} -MaxAgeSeconds 45 "
                    + "-StartupGraceSeconds 120 -ProcessAgeSeconds 500",
                    "Write-Output ('healthy=' + $healthy)",
                    "Write-Output ('missing=' + $missing)",
                ]
            )
            completed = subprocess.run(
                ["powershell.exe", "-NoProfile", "-Command", command],
                capture_output=True,
                encoding="utf-8",
                errors="replace",
                timeout=15,
                check=False,
            )

        output = completed.stdout + completed.stderr
        self.assertEqual(completed.returncode, 0, output)
        self.assertIn("healthy=", completed.stdout)
        self.assertIn("missing=heartbeat_missing", completed.stdout)
        self.assertIn("process_age_seconds=500", completed.stdout)

    def test_watchdog_restores_orphaned_restart_claim(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            repo_root = Path(temp_dir)
            restart_path = repo_root / ".codex_discord_bot.restart"
            claim_path = repo_root / ".codex_discord_bot.restart.claimed.999999"
            claim_text = "identity=42|123"
            _ = claim_path.write_text(claim_text, encoding="ascii")
            command = "\n".join(
                [
                    f"$RestartRequestPath = {str(restart_path)!r}",
                    f"$RestartClaimPattern = {str(repo_root / '.codex_discord_bot.restart.claimed.*')!r}",
                    "$BotScript = 'bot.py'",
                    "$RuntimeLockPath = 'runtime.lock'",
                    "function Write-LauncherLog { param([string]$Message) }",
                    "function Test-RestartClaimMatchesCurrentBot { return $true }",
                    "function Test-WatchdogClaimOwnerAlive { return $false }",
                    f". {str(WATCHDOG_RESTART_RUNTIME)!r}",
                    "Restore-OrphanedRestartClaims",
                ]
            )
            completed = subprocess.run(
                ["powershell.exe", "-NoProfile", "-Command", command],
                capture_output=True,
                encoding="utf-8",
                errors="replace",
                timeout=15,
                check=False,
            )

            self.assertEqual(completed.returncode, 0, completed.stderr)
            self.assertTrue(restart_path.exists())
            self.assertEqual(restart_path.read_text(encoding="ascii"), claim_text)
            self.assertFalse(claim_path.exists())

    def test_orphaned_restart_claim_never_overwrites_primary_request(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            repo_root = Path(temp_dir)
            restart_path = repo_root / ".codex_discord_bot.restart"
            claim_path = repo_root / ".codex_discord_bot.restart.claimed.999999"
            _ = restart_path.write_text("identity=17|200", encoding="ascii")
            _ = claim_path.write_text("identity=17|100", encoding="ascii")
            command = "\n".join(
                [
                    f"$RestartRequestPath = {str(restart_path)!r}",
                    f"$RestartClaimPattern = {str(repo_root / '.codex_discord_bot.restart.claimed.*')!r}",
                    "$BotScript = 'bot.py'",
                    "$RuntimeLockPath = 'runtime.lock'",
                    "function Write-LauncherLog { param([string]$Message) }",
                    "function Test-RestartClaimMatchesCurrentBot { return $false }",
                    "function Test-WatchdogClaimOwnerAlive { return $false }",
                    f". {str(WATCHDOG_RESTART_RUNTIME)!r}",
                    "Restore-OrphanedRestartClaims",
                ]
            )
            completed = subprocess.run(
                ["powershell.exe", "-NoProfile", "-Command", command],
                capture_output=True,
                encoding="utf-8",
                errors="replace",
                timeout=15,
                check=False,
            )

            self.assertEqual(completed.returncode, 0, completed.stderr)
            self.assertEqual(
                restart_path.read_text(encoding="ascii"),
                "identity=17|200",
            )
            self.assertFalse(claim_path.exists())

    def test_watchdog_restores_exact_orphaned_stop_claim(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            repo_root = Path(temp_dir)
            stop_path = repo_root / ".codex_discord_bot.stop"
            claim_path = repo_root / ".codex_discord_bot.stop.claimed.999999"
            _ = claim_path.write_text("identity=17|100", encoding="ascii")
            command = "\n".join(
                [
                    f"$StopRequestPath = {str(stop_path)!r}",
                    f"$StopClaimPattern = {str(repo_root / '.codex_discord_bot.stop.claimed.*')!r}",
                    "$BotScript = 'bot.py'",
                    "function Write-LauncherLog { param([string]$Message) }",
                    f". {str(WATCHDOG_IDENTITY_RUNTIME)!r}",
                    "function Get-CodexBotProcessIdentityById { return '17|100' }",
                    f". {str(WATCHDOG_STOP_RUNTIME)!r}",
                    "Restore-OrphanedStopClaims",
                ]
            )
            completed = subprocess.run(
                ["powershell.exe", "-NoProfile", "-Command", command],
                capture_output=True,
                encoding="utf-8",
                errors="replace",
                timeout=15,
                check=False,
            )

            self.assertEqual(completed.returncode, 0, completed.stderr)
            self.assertEqual(stop_path.read_text(encoding="ascii"), "identity=17|100")
            self.assertFalse(claim_path.exists())

    def test_status_reports_codex_app_package_update_detection(self) -> None:
        text = PLUGIN_STATUS.read_text(encoding="utf-8")

        self.assertIn("Get-AppxPackage -Name OpenAI.Codex", text)
        self.assertIn("[System.IO.File]::ReadAllText", text)
        self.assertIn("[System.Text.UTF8Encoding]::new($false)", text)
        self.assertIn("[System.IO.File]::WriteAllText", text)
        self.assertIn("codex_app_package_version:", text)
        self.assertIn("codex_app_update_detected:", text)
        self.assertIn("codex_app_restart_recommended:", text)

    def test_status_preserves_utf8_state_when_recording_codex_app_update(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            repo_root = Path(temp_dir) / "repo"
            repo_root.mkdir()
            _ = (repo_root / PYTHON_RUNTIME.name).write_text(
                PYTHON_RUNTIME.read_text(encoding="utf-8"),
                encoding="utf-8",
            )
            state_path = Path(temp_dir) / "bridge_state.json"
            non_ascii_note = "\ud55c\uae00"
            _ = state_path.write_text(
                json.dumps(
                    {
                        "note": non_ascii_note,
                        "codex_app_package_version": "1.0.0.0",
                    },
                    ensure_ascii=False,
                ),
                encoding="utf-8",
            )
            command = "\n".join(
                [
                    "function Get-AppxPackage {",
                    "    [CmdletBinding()]",
                    "    param([string]$Name)",
                    "    [pscustomobject]@{ Version = '2.0.0.0' }",
                    "}",
                    f"& {str(PLUGIN_STATUS)!r} -RepoRoot {str(repo_root)!r}",
                ]
            )
            env = os.environ.copy()
            env["CODEX_BRIDGE_STATE"] = str(state_path)
            env["CODEX_DISCORD_PYTHON"] = sys.executable
            env["CODEX_DISCORD_RUNTIME"] = "python"

            completed = subprocess.run(
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
                env=env,
                timeout=30,
                check=False,
            )

            self.assertEqual(completed.returncode, 0, completed.stderr)
            self.assertIn(
                "codex_app_previous_package_version: 1.0.0.0", completed.stdout
            )
            self.assertIn("codex_app_update_detected: True", completed.stdout)
            state_bytes = state_path.read_bytes()
            self.assertFalse(state_bytes.startswith(b"\xef\xbb\xbf"))
            state_text = state_bytes.decode("utf-8")
            self.assertIn(non_ascii_note, state_text)
            self.assertIn('"codex_app_package_version"', state_text)
            self.assertIn('"2.0.0.0"', state_text)


if __name__ == "__main__":
    _ = unittest.main()
