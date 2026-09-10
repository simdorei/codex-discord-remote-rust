from __future__ import annotations

import json

from simdorei_mcp_common.messages import GatewayCommand


def canonical_command_json(
    command: GatewayCommand,
    *,
    exclude: set[str],
) -> str:
    payload = command.model_dump(mode="json", exclude=exclude)
    return json.dumps(
        payload,
        ensure_ascii=False,
        separators=(",", ":"),
        sort_keys=True,
    )


__all__ = ["canonical_command_json"]
