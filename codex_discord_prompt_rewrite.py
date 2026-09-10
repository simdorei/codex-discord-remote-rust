from __future__ import annotations

import hashlib
import xml.etree.ElementTree as element_tree
from collections.abc import Callable
from pathlib import Path
from typing import Final

import codex_discord_prompt_mapped_delivery as mapped_delivery
from codex_pro_runtime_preflight import (
    ProRuntimePreflightError,
    ProRuntimeStatus,
    run_pro_runtime_preflight_with_recovery,
)
from codex_pro_runtime_diagnostics import (
    ProDiagnosticCode,
    ProDiagnosticStage,
    diagnostic,
)
from codex_pro_prompt_contract import PRO_SKILL_CALL
from codex_remote_mcp_binding import connect_remote_mcp_device
from codex_remote_mcp_bridge_config import (
    DeviceTicket,
    RemoteMcpConfigurationError,
)
from codex_remote_mcp_bridge_connection import RemoteMcpBridgeError
from simdorei_mcp_common.connector_contract import (
    PRODUCTION_CONNECTOR_NAME,
    PRODUCTION_CONNECTOR_RESOURCE,
)

LogFunc = Callable[[str], None]
DeviceConnector = Callable[[Path, LogFunc], DeviceTicket | None]
RuntimePreflight = Callable[[], ProRuntimeStatus]
PRO_CONVERSATION_SCOPE_LENGTH: Final = 24
PRO_REVIEW_MARKER: Final = "<pro-review>"


def _rewrite_pro_command(prompt: str) -> str | None:
    parts = prompt.split(maxsplit=1)
    if not parts or parts[0].casefold() != "!pro":
        return None

    request = parts[1] if len(parts) == 2 else ""
    review_parts = request.split(maxsplit=1)
    if review_parts and review_parts[0].casefold() == "review":
        review_request = review_parts[1] if len(review_parts) == 2 else ""
        suffix = f"\n{review_request}" if review_request else ""
        return f"{PRO_SKILL_CALL} {PRO_REVIEW_MARKER}{suffix}"

    return f"{PRO_SKILL_CALL} {request}".rstrip()


def is_pro_command(prompt: str) -> bool:
    return _rewrite_pro_command(prompt) is not None


def pro_conversation_scope(thread_id: str) -> str:
    """Return a stable opaque browser-chat scope for one Codex thread."""
    digest = hashlib.sha256(thread_id.encode("utf-8")).hexdigest()
    return f"codex-pro-{digest[:PRO_CONVERSATION_SCOPE_LENGTH]}"


def rewrite_prompt(
    prompt: str,
    target_thread_id: str | None = None,
    *,
    cwd: Path,
    log: LogFunc,
    device_connector: DeviceConnector = connect_remote_mcp_device,
    runtime_preflight: RuntimePreflight = run_pro_runtime_preflight_with_recovery,
) -> mapped_delivery.PromptPreprocessResult:
    rewritten = _rewrite_pro_command(prompt)
    if rewritten is None:
        return mapped_delivery.keep_prompt(prompt)
    try:
        _ = runtime_preflight()
    except ProRuntimePreflightError as exc:
        return mapped_delivery.block_prompt(exc.diagnostic)
    if not target_thread_id:
        return mapped_delivery.keep_prompt(rewritten)
    try:
        ticket = device_connector(cwd, log)
    except RemoteMcpConfigurationError as exc:
        return mapped_delivery.block_prompt(
            diagnostic(
                stage=ProDiagnosticStage.REMOTE_MCP,
                code=ProDiagnosticCode.REMOTE_MCP_CONFIGURATION_INVALID,
                public_message="The local PC connection is configured incorrectly.",
                recovery_action="Repair the remote MCP configuration, restart the remote bot, then retry !pro.",
                internal_detail=str(exc),
            )
        )
    except RemoteMcpBridgeError as exc:
        return mapped_delivery.block_prompt(
            diagnostic(
                stage=ProDiagnosticStage.REMOTE_MCP,
                code=ProDiagnosticCode.REMOTE_MCP_CONNECTION_FAILED,
                public_message="The local PC connection did not become ready.",
                recovery_action="Restart the remote bot, verify remote MCP connectivity, then retry !pro.",
                internal_detail=str(exc),
            )
        )
    if ticket is None:
        return mapped_delivery.block_prompt(
            diagnostic(
                stage=ProDiagnosticStage.REMOTE_MCP,
                code=ProDiagnosticCode.REMOTE_MCP_NOT_CONFIGURED,
                public_message="The local PC connection is not configured.",
                recovery_action="Configure remote MCP, restart the remote bot, then retry !pro.",
                internal_detail="remote MCP is not configured",
            )
        )
    device_instruction = element_tree.Element(
        "local-device-mcp",
        {
            "connector": PRODUCTION_CONNECTOR_NAME,
            "resource": PRODUCTION_CONNECTOR_RESOURCE,
            "device_id": ticket.device_id,
            "working_directory": str(ticket.working_directory),
            "conversation_scope": pro_conversation_scope(target_thread_id),
        },
    )
    device_instruction.text = "\n".join(
        (
            "",
            "Use only the connector named in this tag and select it explicitly.",
            "Use PC mode by default.",
            "Call list_devices, verify that device_id is online, then call select_device",
            "exactly once with the device_id, working_directory, and connector resource",
            "from this tag. The working directory identifies the project for this ticket.",
            "Read a file before updating it and pass its SHA-256 when writing an existing file.",
            "",
        )
    )
    instruction = element_tree.tostring(device_instruction, encoding="unicode")
    return mapped_delivery.keep_prompt(f"{rewritten}\n{instruction}")
