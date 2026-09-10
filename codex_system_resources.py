from __future__ import annotations

import os
import re
import subprocess
from dataclasses import dataclass
from pathlib import Path
from typing import Final, override

PROBE_TIMEOUT_SECONDS: Final = 10
BYTES_PER_KIB: Final = 1024
DRIVE_LABEL_RE: Final = re.compile(r"^[A-Za-z]:$")


@dataclass(frozen=True, slots=True)
class ResourceProbeError(RuntimeError):
    detail: str

    @override
    def __str__(self) -> str:
        return self.detail


@dataclass(frozen=True, slots=True)
class SystemResourceSnapshot:
    cpu_percent: float | None
    memory_total_bytes: int
    memory_free_bytes: int
    disk_label: str
    disk_total_bytes: int
    disk_free_bytes: int


def build_system_resources_message(root: Path | None = None) -> str:
    try:
        return format_system_resources(read_system_resources(root))
    except ResourceProbeError as exc:
        return f"System resources failed\n\nERROR: {exc}"


def read_system_resources(root: Path | None = None) -> SystemResourceSnapshot:
    drive_label = resolve_drive_label(root)
    try:
        completed = subprocess.run(
            [
                "powershell.exe",
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                build_powershell_resource_script(drive_label),
            ],
            capture_output=True,
            check=False,
            encoding="utf-8",
            errors="replace",
            text=True,
            timeout=PROBE_TIMEOUT_SECONDS,
        )
    except subprocess.TimeoutExpired as exc:
        raise ResourceProbeError(f"PowerShell probe timed out after {PROBE_TIMEOUT_SECONDS}s") from exc
    if completed.returncode != 0:
        detail = (completed.stderr or completed.stdout or "unknown error").strip()
        raise ResourceProbeError(detail)
    return parse_powershell_resource_metrics(completed.stdout)


def resolve_drive_label(root: Path | None = None) -> str:
    raw_label = (root or Path.cwd()).resolve().anchor.rstrip("\\/")
    if not raw_label:
        raw_label = os.environ.get("SystemDrive", "C:").rstrip("\\/")
    if DRIVE_LABEL_RE.fullmatch(raw_label) is None:
        raise ResourceProbeError(f"Unsupported disk root: {raw_label}")
    return raw_label.upper()


def build_powershell_resource_script(drive_label: str) -> str:
    if DRIVE_LABEL_RE.fullmatch(drive_label) is None:
        raise ResourceProbeError(f"Unsupported disk root: {drive_label}")
    return "\n".join(
        [
            "$ErrorActionPreference = 'Stop'",
            "$cpu = (Get-CimInstance Win32_Processor | Measure-Object -Property LoadPercentage -Average).Average",
            "$os = Get-CimInstance Win32_OperatingSystem",
            f"$disk = Get-CimInstance Win32_LogicalDisk -Filter \"DeviceID='{drive_label}'\"",
            f"if ($null -eq $disk) {{ throw 'Logical disk {drive_label} not found' }}",
            "if ($null -eq $cpu) { 'cpu_percent=' } else { 'cpu_percent=' + [math]::Round([double]$cpu, 1) }",
            "'memory_total_kib=' + $os.TotalVisibleMemorySize",
            "'memory_free_kib=' + $os.FreePhysicalMemory",
            "'disk_label=' + $disk.DeviceID",
            "'disk_total_bytes=' + $disk.Size",
            "'disk_free_bytes=' + $disk.FreeSpace",
        ]
    )


def parse_powershell_resource_metrics(text: str) -> SystemResourceSnapshot:
    values: dict[str, str] = {}
    for raw_line in text.splitlines():
        key, separator, value = raw_line.partition("=")
        if separator:
            values[key.strip()] = value.strip()
    return SystemResourceSnapshot(
        cpu_percent=parse_optional_float(values, "cpu_percent"),
        memory_total_bytes=parse_required_int(values, "memory_total_kib") * BYTES_PER_KIB,
        memory_free_bytes=parse_required_int(values, "memory_free_kib") * BYTES_PER_KIB,
        disk_label=parse_required_text(values, "disk_label"),
        disk_total_bytes=parse_required_int(values, "disk_total_bytes"),
        disk_free_bytes=parse_required_int(values, "disk_free_bytes"),
    )


def format_system_resources(snapshot: SystemResourceSnapshot) -> str:
    return "\n".join(
        [
            "System resources",
            f"cpu: {format_optional_percent(snapshot.cpu_percent)}",
            (
                f"ram_free: {format_bytes(snapshot.memory_free_bytes)} / "
                f"{format_bytes(snapshot.memory_total_bytes)} "
                f"({format_free_percent(snapshot.memory_free_bytes, snapshot.memory_total_bytes)} free)"
            ),
            (
                f"disk_free {snapshot.disk_label} {format_bytes(snapshot.disk_free_bytes)} / "
                f"{format_bytes(snapshot.disk_total_bytes)} "
                f"({format_free_percent(snapshot.disk_free_bytes, snapshot.disk_total_bytes)} free)"
            ),
        ]
    )


def parse_required_text(values: dict[str, str], key: str) -> str:
    value = values.get(key, "")
    if not value:
        raise ResourceProbeError(f"Missing resource metric: {key}")
    return value


def parse_required_int(values: dict[str, str], key: str) -> int:
    value = parse_required_text(values, key)
    try:
        return int(value)
    except ValueError as exc:
        raise ResourceProbeError(f"Invalid integer resource metric {key}: {value}") from exc


def parse_optional_float(values: dict[str, str], key: str) -> float | None:
    value = values.get(key, "")
    if not value:
        return None
    try:
        return float(value)
    except ValueError as exc:
        raise ResourceProbeError(f"Invalid float resource metric {key}: {value}") from exc


def format_optional_percent(value: float | None) -> str:
    if value is None:
        return "unknown"
    return f"{value:.1f}%"


def format_free_percent(free_bytes: int, total_bytes: int) -> str:
    if total_bytes <= 0:
        return "unknown"
    return f"{(free_bytes / total_bytes) * 100:.1f}%"


def format_bytes(value: int) -> str:
    amount = float(value)
    for unit in ("B", "KiB", "MiB", "GiB", "TiB"):
        if amount < BYTES_PER_KIB or unit == "TiB":
            return f"{amount:.1f} {unit}"
        amount /= BYTES_PER_KIB
    raise ResourceProbeError(f"Unsupported byte value: {value}")
