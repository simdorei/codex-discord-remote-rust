from __future__ import annotations

import hmac
import json
import traceback

import pytest

from remote_mcp_server.simdorei_mcp.device_credentials import (
    DeviceAuthenticator,
    DeviceCredentialRegistry,
)
from remote_mcp_server.simdorei_mcp.settings import (
    GatewaySettingsError,
    load_gateway_settings,
)
from simdorei_mcp_common.messages import DeviceId

TOKEN_A = "a" * 40
TOKEN_B = "b" * 40


def _registry(*devices: tuple[str, str]) -> str:
    return json.dumps(
        {
            "version": 1,
            "devices": [
                {"device_id": device_id, "token": token} for device_id, token in devices
            ],
        }
    )


def _set_required_environment(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv(
        "SIMDOREI_MCP_PUBLIC_BASE_URL",
        "https://simdorei.duckdns.org",
    )
    monkeypatch.setenv(
        "SIMDOREI_MCP_OWNER_TOKEN",
        "owner-secret-12345678901234567890",
    )


def test_loads_multi_device_registry_with_exact_legacy_overlap(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    _set_required_environment(monkeypatch)
    monkeypatch.setenv(
        "SIMDOREI_MCP_DEVICE_CREDENTIALS_JSON",
        _registry(("office-pc", TOKEN_A), ("laptop-pc", TOKEN_B)),
    )
    monkeypatch.setenv("SIMDOREI_MCP_DEVICE_ID", "office-pc")
    monkeypatch.setenv("SIMDOREI_MCP_DEVICE_TOKEN", TOKEN_A)

    settings = load_gateway_settings()

    assert settings.device_credentials.version == 1
    assert tuple(
        credential.device_id for credential in settings.device_credentials.devices
    ) == (DeviceId("office-pc"), DeviceId("laptop-pc"))
    assert TOKEN_A not in repr(settings)
    assert TOKEN_B not in repr(settings)


def test_loads_legacy_single_device_pair_without_registry(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    _set_required_environment(monkeypatch)
    monkeypatch.delenv("SIMDOREI_MCP_DEVICE_CREDENTIALS_JSON", raising=False)
    monkeypatch.setenv("SIMDOREI_MCP_DEVICE_ID", "office-pc")
    monkeypatch.setenv("SIMDOREI_MCP_DEVICE_TOKEN", TOKEN_A)

    settings = load_gateway_settings()

    assert len(settings.device_credentials.devices) == 1
    assert settings.device_credentials.devices[0].device_id == DeviceId("office-pc")


def test_rejects_oversized_registry_before_parsing(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    _set_required_environment(monkeypatch)
    monkeypatch.setenv("SIMDOREI_MCP_DEVICE_CREDENTIALS_JSON", "{" + "x" * 24_576)
    monkeypatch.delenv("SIMDOREI_MCP_DEVICE_ID", raising=False)
    monkeypatch.delenv("SIMDOREI_MCP_DEVICE_TOKEN", raising=False)

    with pytest.raises(GatewaySettingsError, match="exceeds 24 KiB"):
        _ = load_gateway_settings()


def test_loads_thirty_two_devices_with_maximum_sized_tokens(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    # Given the approved registration capacity at the token-size boundary.
    _set_required_environment(monkeypatch)
    devices = tuple((f"pc-{index}", f"{index:02d}" * 256) for index in range(32))
    monkeypatch.setenv("SIMDOREI_MCP_DEVICE_CREDENTIALS_JSON", _registry(*devices))
    monkeypatch.delenv("SIMDOREI_MCP_DEVICE_ID", raising=False)
    monkeypatch.delenv("SIMDOREI_MCP_DEVICE_TOKEN", raising=False)

    # When the gateway loads its device registry.
    settings = load_gateway_settings()

    # Then all thirty-two devices are available for authentication.
    assert len(settings.device_credentials.devices) == 32


@pytest.mark.parametrize("use_registry", [True, False])
def test_invalid_credentials_do_not_leak_tokens_through_exception_chaining(
    monkeypatch: pytest.MonkeyPatch,
    use_registry: bool,
) -> None:
    _set_required_environment(monkeypatch)
    secret_token = "private-token-that-must-never-reach-startup-logs"
    if use_registry:
        monkeypatch.setenv(
            "SIMDOREI_MCP_DEVICE_CREDENTIALS_JSON",
            '{"version":1,"devices":[{"device_id":"office-pc","token":"' + secret_token,
        )
        monkeypatch.delenv("SIMDOREI_MCP_DEVICE_ID", raising=False)
        monkeypatch.delenv("SIMDOREI_MCP_DEVICE_TOKEN", raising=False)
    else:
        monkeypatch.delenv("SIMDOREI_MCP_DEVICE_CREDENTIALS_JSON", raising=False)
        monkeypatch.setenv("SIMDOREI_MCP_DEVICE_ID", "office-pc")
        monkeypatch.setenv("SIMDOREI_MCP_DEVICE_TOKEN", secret_token + " ")

    with pytest.raises(GatewaySettingsError) as captured:
        _ = load_gateway_settings()

    rendered_traceback = "".join(traceback.format_exception(captured.value))
    assert secret_token not in rendered_traceback


@pytest.mark.parametrize(
    ("device_id", "token", "message"),
    [
        ("office-pc", "", "must be set together"),
        ("", TOKEN_A, "must be set together"),
        ("unknown-pc", TOKEN_A, "must exactly match"),
        ("office-pc", TOKEN_B, "must exactly match"),
    ],
)
def test_rejects_partial_or_mismatched_legacy_credentials(
    monkeypatch: pytest.MonkeyPatch,
    device_id: str,
    token: str,
    message: str,
) -> None:
    _set_required_environment(monkeypatch)
    monkeypatch.setenv(
        "SIMDOREI_MCP_DEVICE_CREDENTIALS_JSON",
        _registry(("office-pc", TOKEN_A)),
    )
    monkeypatch.setenv("SIMDOREI_MCP_DEVICE_ID", device_id)
    monkeypatch.setenv("SIMDOREI_MCP_DEVICE_TOKEN", token)

    with pytest.raises(GatewaySettingsError, match=message):
        _ = load_gateway_settings()


@pytest.mark.parametrize(
    "payload",
    [
        {
            "version": 1,
            "devices": [
                {"device_id": "same-pc", "token": TOKEN_A},
                {"device_id": "same-pc", "token": TOKEN_B},
            ],
        },
        {
            "version": 1,
            "devices": [
                {"device_id": "office-pc", "token": TOKEN_A},
                {"device_id": "laptop-pc", "token": TOKEN_A},
            ],
        },
        {
            "version": 1,
            "devices": [
                {"device_id": "invalid pc", "token": TOKEN_A},
            ],
        },
        {
            "version": 1,
            "devices": [
                {"device_id": "office-pc", "token": "short"},
            ],
        },
        {
            "version": 1,
            "devices": [
                {"device_id": "office-pc", "token": "x" * 39 + " "},
            ],
        },
        {
            "version": 1,
            "devices": [
                {"device_id": "office-pc", "token": "가" * 40},
            ],
        },
        {
            "version": 1,
            "devices": [
                {"device_id": f"pc-{index}", "token": str(index) * 40}
                for index in range(33)
            ],
        },
    ],
)
def test_registry_rejects_ambiguous_or_unsupported_credentials(
    payload: object,  # noqa: ANN401  # noqa: OBJECT_OK - parametrized invalid input shapes.
) -> None:
    with pytest.raises(ValueError):
        _ = DeviceCredentialRegistry.model_validate(payload)


def test_authenticator_scans_every_digest_without_early_return(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    registry = DeviceCredentialRegistry.model_validate_json(
        _registry(("office-pc", TOKEN_A), ("laptop-pc", TOKEN_B))
    )
    authenticator = DeviceAuthenticator(registry)
    original_compare = hmac.compare_digest
    compared: list[tuple[bytes, bytes]] = []

    def record_compare(candidate: bytes, expected: bytes) -> bool:
        compared.append((candidate, expected))
        return original_compare(candidate, expected)

    monkeypatch.setattr(hmac, "compare_digest", record_compare)

    assert authenticator.authenticate(f"Bearer {TOKEN_A}") == DeviceId("office-pc")
    assert len(compared) == 2
    compared.clear()
    assert authenticator.authenticate(f"Bearer {'z' * 40}") is None
    assert len(compared) == 2
    assert authenticator.authenticate(TOKEN_A) is None
