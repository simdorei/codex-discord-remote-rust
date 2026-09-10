from __future__ import annotations

from typing import cast

import pytest

from remote_mcp_server.simdorei_mcp import broker_results
from simdorei_mcp_common.messages import BridgeResult


def _invalid_result() -> BridgeResult:
    return cast(BridgeResult, object())


def test_project_info_output_rejects_an_invalid_runtime_result() -> None:
    with pytest.raises(AssertionError):
        _ = broker_results.project_info_output(_invalid_result())


def test_list_files_output_rejects_an_invalid_runtime_result() -> None:
    with pytest.raises(AssertionError):
        _ = broker_results.list_files_output(_invalid_result())


def test_read_file_output_rejects_an_invalid_runtime_result() -> None:
    with pytest.raises(AssertionError):
        _ = broker_results.read_file_output(_invalid_result())


def test_write_file_output_rejects_an_invalid_runtime_result() -> None:
    with pytest.raises(AssertionError):
        _ = broker_results.write_file_output(_invalid_result())


def test_operation_output_rejects_an_invalid_runtime_result() -> None:
    with pytest.raises(AssertionError):
        _ = broker_results.operation_output(_invalid_result())


def test_project_session_result_rejects_an_invalid_runtime_result() -> None:
    with pytest.raises(AssertionError):
        _ = broker_results.require_project_session_result(_invalid_result())
