from __future__ import annotations

from dataclasses import dataclass
from enum import StrEnum
from typing import override


class ProDiagnosticStage(StrEnum):
    PLUGIN_INVENTORY = "plugin_inventory"
    PLUGIN_MANIFEST = "plugin_manifest"
    PLUGIN_CONTENT = "plugin_content"
    RESIDENT_APP_SERVER = "resident_app_server"
    REMOTE_MCP = "remote_mcp"
    PROJECT_TICKET = "project_ticket"


class ProDiagnosticCode(StrEnum):
    PLUGIN_INVENTORY_QUERY_FAILED = "plugin_inventory_query_failed"
    PLUGIN_INVENTORY_INVALID = "plugin_inventory_invalid"
    REMOTE_PLUGIN_MISSING = "remote_plugin_missing"
    REMOTE_PLUGIN_NOT_INSTALLED = "remote_plugin_not_installed"
    REMOTE_PLUGIN_DISABLED = "remote_plugin_disabled"
    REMOTE_PLUGIN_VERSION_INVALID = "remote_plugin_version_invalid"
    REMOTE_PLUGIN_VERSION_MISMATCH = "remote_plugin_version_mismatch"
    BROWSER_PLUGIN_MISSING = "browser_plugin_missing"
    BROWSER_PLUGIN_NOT_INSTALLED = "browser_plugin_not_installed"
    BROWSER_PLUGIN_DISABLED = "browser_plugin_disabled"
    BROWSER_PLUGIN_VERSION_INVALID = "browser_plugin_version_invalid"
    PLUGIN_CONTENT_UNVERIFIED = "plugin_content_unverified"
    REMOTE_MANIFEST_UNAVAILABLE = "remote_manifest_unavailable"
    REMOTE_MANIFEST_INVALID = "remote_manifest_invalid"
    RESIDENT_UNHEALTHY = "resident_unhealthy"
    RESIDENT_SNAPSHOT_FAILED = "resident_snapshot_failed"
    RESIDENT_SNAPSHOT_MISSING = "resident_snapshot_missing"
    RESIDENT_STALE = "resident_stale"
    REMOTE_MCP_CONFIGURATION_INVALID = "remote_mcp_configuration_invalid"
    REMOTE_MCP_CONNECTION_FAILED = "remote_mcp_connection_failed"
    REMOTE_MCP_NOT_CONFIGURED = "remote_mcp_not_configured"
    PROJECT_TICKET_TIMEZONE_INVALID = "project_ticket_timezone_invalid"
    PROJECT_TICKET_EXPIRED = "project_ticket_expired"


@dataclass(frozen=True, slots=True)
class ProRuntimeDiagnostic:
    stage: ProDiagnosticStage
    code: ProDiagnosticCode
    public_message: str
    recovery_action: str
    internal_detail: str


@dataclass(frozen=True, slots=True)
class ProRuntimePreflightError(Exception):
    diagnostic: ProRuntimeDiagnostic

    @override
    def __str__(self) -> str:
        return self.diagnostic.internal_detail


def diagnostic(
    *,
    stage: ProDiagnosticStage,
    code: ProDiagnosticCode,
    public_message: str,
    recovery_action: str,
    internal_detail: str,
) -> ProRuntimeDiagnostic:
    return ProRuntimeDiagnostic(
        stage=stage,
        code=code,
        public_message=public_message,
        recovery_action=recovery_action,
        internal_detail=internal_detail,
    )


def preflight_error(
    *,
    stage: ProDiagnosticStage,
    code: ProDiagnosticCode,
    public_message: str,
    recovery_action: str,
    internal_detail: str,
) -> ProRuntimePreflightError:
    return ProRuntimePreflightError(
        diagnostic(
            stage=stage,
            code=code,
            public_message=public_message,
            recovery_action=recovery_action,
            internal_detail=internal_detail,
        )
    )
