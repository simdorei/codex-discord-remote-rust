from __future__ import annotations

import pytest
from pydantic import ValidationError

from simdorei_mcp_common.messages import BridgeHello, DeviceId, parse_bridge_message


def test_protocol_ten_rejects_an_old_protocol_nine_bridge() -> None:
    current = BridgeHello(
        protocol_version=10,
        device_id=DeviceId("device-a"),
    ).model_dump_json()
    assert isinstance(parse_bridge_message(current), BridgeHello)

    with pytest.raises(ValidationError):
        _ = parse_bridge_message(
            '{"type":"hello","protocol_version":9,"device_id":"device-a"}'
        )
