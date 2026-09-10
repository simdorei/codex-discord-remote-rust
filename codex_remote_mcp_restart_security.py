from __future__ import annotations

import base64
import binascii
import subprocess
import sys
from pathlib import Path
from typing import Final

from codex_remote_mcp_restart_models import RestartHandoffError

_CREATE_NO_WINDOW: Final = 0x08000000
_DPAPI_PROTECT_SCRIPT: Final = (
    "Add-Type -AssemblyName System.Security;"
    "$data=[Convert]::FromBase64String([Console]::In.ReadToEnd());"
    "$value=[Security.Cryptography.ProtectedData]::Protect("
    "$data,$null,[Security.Cryptography.DataProtectionScope]::CurrentUser);"
    "[Console]::Out.Write([Convert]::ToBase64String($value))"
)
_DPAPI_UNPROTECT_SCRIPT: Final = (
    "Add-Type -AssemblyName System.Security;"
    "$data=[Convert]::FromBase64String([Console]::In.ReadToEnd());"
    "$value=[Security.Cryptography.ProtectedData]::Unprotect("
    "$data,$null,[Security.Cryptography.DataProtectionScope]::CurrentUser);"
    "[Console]::Out.Write([Convert]::ToBase64String($value))"
)
_PRIVATE_DIRECTORY_SCRIPT: Final = (
    "$path=[Console]::In.ReadToEnd();"
    "$acl=New-Object System.Security.AccessControl.DirectorySecurity;"
    "$acl.SetAccessRuleProtection($true,$false);"
    "$sids=@([Security.Principal.WindowsIdentity]::GetCurrent().User,"
    "(New-Object Security.Principal.SecurityIdentifier('S-1-5-18')));"
    "foreach($sid in $sids){"
    "$rule=New-Object System.Security.AccessControl.FileSystemAccessRule("
    "$sid,'FullControl','ContainerInherit,ObjectInherit','None','Allow');"
    "$acl.AddAccessRule($rule)};"
    "[IO.Directory]::SetAccessControl($path,$acl)"
)


class WindowsDpapiProtector:
    def protect(self, payload: bytes) -> bytes:
        return _run_powershell_binary(_DPAPI_PROTECT_SCRIPT, payload)

    def unprotect(self, payload: bytes) -> bytes:
        return _run_powershell_binary(_DPAPI_UNPROTECT_SCRIPT, payload)


def secure_handoff_directory(path: Path) -> None:
    if sys.platform != "win32":
        raise RestartHandoffError("Secure restart handoff storage requires Windows.")
    try:
        completed = subprocess.run(
            [
                "powershell.exe",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                _PRIVATE_DIRECTORY_SCRIPT,
            ],
            input=str(path),
            capture_output=True,
            check=False,
            text=True,
            timeout=15,
            creationflags=_CREATE_NO_WINDOW,
        )
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise RestartHandoffError(
            f"Restart handoff ACL setup failed: {type(exc).__name__}."
        ) from exc
    if completed.returncode != 0:
        detail = completed.stderr.strip().replace("\r", " ").replace("\n", " ")[:200]
        raise RestartHandoffError(
            f"Restart handoff ACL setup failed with exit code "
            + f"{completed.returncode}: {detail}"
        )


def _run_powershell_binary(script: str, payload: bytes) -> bytes:
    if sys.platform != "win32":
        raise RestartHandoffError("Windows DPAPI is unavailable on this platform.")
    try:
        completed = subprocess.run(
            ["powershell.exe", "-NoProfile", "-NonInteractive", "-Command", script],
            input=base64.b64encode(payload).decode("ascii"),
            capture_output=True,
            check=False,
            text=True,
            timeout=15,
            creationflags=_CREATE_NO_WINDOW,
        )
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise RestartHandoffError(
            f"Windows DPAPI process failed: {type(exc).__name__}."
        ) from exc
    if completed.returncode != 0:
        detail = completed.stderr.strip().replace("\r", " ").replace("\n", " ")[:200]
        raise RestartHandoffError(
            f"Windows DPAPI failed with exit code {completed.returncode}: {detail}"
        )
    try:
        return base64.b64decode(completed.stdout.strip(), validate=True)
    except binascii.Error as exc:
        raise RestartHandoffError("Windows DPAPI returned invalid data.") from exc


__all__ = ["WindowsDpapiProtector", "secure_handoff_directory"]
