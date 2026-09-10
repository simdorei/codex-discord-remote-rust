from __future__ import annotations

import hashlib

from simdorei_mcp_common.messages import (
    GatewayCommand,
    ProjectOperationCommand,
    RequestId,
)
from simdorei_mcp_common.canonical_json import canonical_command_json


def command_fingerprint(command: GatewayCommand) -> str:
    payload = canonical_command_json(command, exclude={"deadline_at"})
    return hashlib.sha256(payload.encode("utf-8")).hexdigest()


def derive_project_operation_request_id(
    base_request_id: RequestId,
    command: ProjectOperationCommand,
) -> RequestId:
    command_payload = canonical_command_json(
        command,
        exclude={"deadline_at", "request_id"},
    )
    payload = (
        "project-operation-request-id:v1\n"
        + f"{len(base_request_id)}:{base_request_id}\n"
        + command_payload
    )
    return RequestId(hashlib.sha256(payload.encode("utf-8")).hexdigest())


__all__ = ["command_fingerprint", "derive_project_operation_request_id"]
