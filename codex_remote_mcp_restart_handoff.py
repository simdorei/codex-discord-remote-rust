from __future__ import annotations

import base64
import binascii
import hashlib
import os
import secrets
from collections.abc import Callable
from datetime import UTC, datetime, timedelta
from pathlib import Path
from typing import ClassVar, Final

from pydantic import BaseModel, ConfigDict, ValidationError

from codex_remote_mcp_bridge_config import RemoteMcpBridgeConfig
from codex_remote_mcp_restart_models import (
    HandoffProtector,
    RestartHandoffError,
    RestartProject,
)
from codex_remote_mcp_restart_security import (
    WindowsDpapiProtector,
    secure_handoff_directory,
)
from simdorei_mcp_common.messages import ProjectUpsert

HANDOFF_FORMAT_VERSION: Final = 1
HANDOFF_PROTOCOL_VERSION: Final = 10
HANDOFF_TTL_SECONDS: Final = 120
RESUME_ENV_NAME: Final = "CODEX_REMOTE_MCP_RESTART_RESUME"


class _StoredProject(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True)

    project: ProjectUpsert
    root: Path


class _RestartPayload(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True)

    format_version: int
    protocol_version: int
    gateway_fingerprint: str
    created_at: datetime
    resume_until: datetime
    projects: tuple[_StoredProject, ...]


class _HandoffEnvelope(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True)

    format_version: int
    ciphertext: str


NowFactory = Callable[[], datetime]
SecureDirectory = Callable[[Path], None]


def restart_handoff_path() -> Path:
    local_data = os.environ.get("LOCALAPPDATA", "").strip()
    base = Path(local_data) if local_data else Path.home() / ".local" / "state"
    repo_digest = hashlib.sha256(
        str(Path(__file__).resolve().parent).casefold().encode("utf-8")
    ).hexdigest()[:16]
    return (
        base
        / "simdorei"
        / "codex-discord-remote"
        / repo_digest
        / "remote-mcp-restart.json"
    )


def write_restart_handoff(
    projects: tuple[RestartProject, ...],
    config: RemoteMcpBridgeConfig,
    *,
    path: Path | None = None,
    protector: HandoffProtector | None = None,
    now: NowFactory = lambda: datetime.now(UTC),
    secure_directory: SecureDirectory | None = None,
) -> bool:
    if not projects:
        return False
    created_at = now()
    payload = _RestartPayload(
        format_version=HANDOFF_FORMAT_VERSION,
        protocol_version=HANDOFF_PROTOCOL_VERSION,
        gateway_fingerprint=_gateway_fingerprint(config),
        created_at=created_at,
        resume_until=created_at + timedelta(seconds=HANDOFF_TTL_SECONDS),
        projects=tuple(
            _StoredProject(project=item.project, root=item.root) for item in projects
        ),
    )
    active_path = path or restart_handoff_path()
    active_path.parent.mkdir(parents=True, exist_ok=True)
    (secure_directory or secure_handoff_directory)(active_path.parent)
    cipher = (protector or WindowsDpapiProtector()).protect(
        payload.model_dump_json().encode("utf-8")
    )
    envelope = _HandoffEnvelope(
        format_version=HANDOFF_FORMAT_VERSION,
        ciphertext=base64.b64encode(cipher).decode("ascii"),
    )
    temporary = active_path.with_name(
        f".{active_path.name}.{secrets.token_hex(8)}.tmp"
    )
    try:
        with temporary.open("xb") as stream:
            _ = stream.write(envelope.model_dump_json().encode("utf-8"))
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, active_path)
    finally:
        temporary.unlink(missing_ok=True)
    return True


def claim_restart_handoff(
    config: RemoteMcpBridgeConfig,
    *,
    path: Path | None = None,
    protector: HandoffProtector | None = None,
    now: NowFactory = lambda: datetime.now(UTC),
) -> tuple[RestartProject, ...]:
    active_path = path or restart_handoff_path()
    if not _resume_requested() or not active_path.is_file():
        return ()
    claimed = active_path.with_name(
        f".{active_path.name}.claimed.{os.getpid()}.{secrets.token_hex(8)}"
    )
    try:
        os.replace(active_path, claimed)
    except FileNotFoundError:
        return ()
    try:
        envelope = _HandoffEnvelope.model_validate_json(
            claimed.read_text(encoding="utf-8")
        )
        if envelope.format_version != HANDOFF_FORMAT_VERSION:
            raise RestartHandoffError("Unsupported restart handoff format.")
        ciphertext = base64.b64decode(envelope.ciphertext, validate=True)
        plaintext = (protector or WindowsDpapiProtector()).unprotect(ciphertext)
        payload = _RestartPayload.model_validate_json(plaintext)
        _validate_payload(payload, config, now())
        return tuple(
            RestartProject(project=item.project, root=item.root.resolve())
            for item in payload.projects
        )
    except (ValidationError, binascii.Error) as exc:
        raise RestartHandoffError("The restart handoff is malformed.") from exc
    finally:
        claimed.unlink(missing_ok=True)


def _resume_requested() -> bool:
    return os.environ.get(RESUME_ENV_NAME, "").strip().casefold() in {
        "1",
        "true",
        "yes",
        "on",
    }


def _validate_payload(
    payload: _RestartPayload,
    config: RemoteMcpBridgeConfig,
    now: datetime,
) -> None:
    if payload.format_version != HANDOFF_FORMAT_VERSION:
        raise RestartHandoffError("Unsupported restart handoff format.")
    if payload.protocol_version != HANDOFF_PROTOCOL_VERSION:
        raise RestartHandoffError("Restart handoff protocol mismatch.")
    if not secrets.compare_digest(
        payload.gateway_fingerprint,
        _gateway_fingerprint(config),
    ):
        raise RestartHandoffError("Restart handoff gateway mismatch.")
    if payload.resume_until <= now:
        raise RestartHandoffError("Restart handoff expired.")
    if not payload.projects:
        raise RestartHandoffError("Restart handoff has no projects.")
    if any(item.project.expires_at <= now for item in payload.projects):
        raise RestartHandoffError("Restart project binding expired.")
    if any(not item.root.resolve().is_dir() for item in payload.projects):
        raise RestartHandoffError("Restart project root is unavailable.")


def _gateway_fingerprint(config: RemoteMcpBridgeConfig) -> str:
    value = f"{config.bridge_url}\0{config.device_id}".encode("utf-8")
    return hashlib.sha256(value).hexdigest()


__all__ = [
    "HandoffProtector",
    "RESUME_ENV_NAME",
    "RestartHandoffError",
    "WindowsDpapiProtector",
    "claim_restart_handoff",
    "restart_handoff_path",
    "write_restart_handoff",
]
