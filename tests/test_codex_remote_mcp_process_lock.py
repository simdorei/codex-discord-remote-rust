from __future__ import annotations

from uuid import uuid4

from codex_remote_mcp_process_lock import acquire_remote_mcp_process_lock


def test_remote_mcp_process_lock_blocks_a_second_owner() -> None:
    device_id = f"test-device-{uuid4().hex}"

    with acquire_remote_mcp_process_lock(device_id) as first:
        with acquire_remote_mcp_process_lock(device_id) as second:
            assert first is True
            assert second is False

    with acquire_remote_mcp_process_lock(device_id) as reacquired:
        assert reacquired is True
