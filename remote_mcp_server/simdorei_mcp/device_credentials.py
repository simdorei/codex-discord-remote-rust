from __future__ import annotations

import hashlib
import hmac
import re
from dataclasses import dataclass
from typing import ClassVar, Literal, Self, final

from pydantic import (
    BaseModel,
    ConfigDict,
    Field,
    SecretStr,
    field_validator,
    model_validator,
)

from simdorei_mcp_common.messages import DeviceId

MAX_DEVICE_CREDENTIALS = 32
MAX_DEVICE_CREDENTIALS_JSON_BYTES = 24 * 1024
_DEVICE_ID_PATTERN = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]{0,127}\Z")


class DeviceCredential(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True, extra="forbid")

    device_id: DeviceId = Field(min_length=1, max_length=128)
    token: SecretStr = Field(min_length=32, max_length=512)

    @field_validator("device_id")
    @classmethod
    def validate_device_id(cls, value: DeviceId) -> DeviceId:
        if _DEVICE_ID_PATTERN.fullmatch(value) is None:
            raise ValueError(  # noqa: EM101  # noqa: GENERIC_ERR_OK - Pydantic validator contract.
                "device_id must use portable ASCII characters"
            )
        return value

    @field_validator("token")
    @classmethod
    def validate_token(cls, value: SecretStr) -> SecretStr:
        token = value.get_secret_value()
        if not token.isascii() or any(
            not 33 <= ord(character) <= 126 for character in token
        ):
            raise ValueError(  # noqa: EM101  # noqa: GENERIC_ERR_OK - Pydantic validator contract.
                "token must contain printable ASCII without whitespace"
            )
        return value


class DeviceCredentialRegistry(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True, extra="forbid")

    version: Literal[1]
    devices: tuple[DeviceCredential, ...] = Field(
        min_length=1,
        max_length=MAX_DEVICE_CREDENTIALS,
    )

    @model_validator(mode="after")
    def reject_ambiguous_credentials(self) -> Self:
        device_ids = [credential.device_id for credential in self.devices]
        token_digests = [
            _token_digest(credential.token.get_secret_value())
            for credential in self.devices
        ]
        if len(device_ids) != len(set(device_ids)):
            raise ValueError(  # noqa: EM101  # noqa: GENERIC_ERR_OK - Pydantic validator contract.
                "device IDs must be unique"
            )
        if len(token_digests) != len(set(token_digests)):
            raise ValueError(  # noqa: EM101  # noqa: GENERIC_ERR_OK - Pydantic validator contract.
                "device tokens must be unique"
            )
        return self


@dataclass(frozen=True, slots=True)
class _DeviceTokenDigest:
    device_id: DeviceId
    digest: bytes


@final
class DeviceAuthenticator:
    def __init__(self, registry: DeviceCredentialRegistry) -> None:
        self._credentials = tuple(
            _DeviceTokenDigest(
                device_id=credential.device_id,
                digest=_token_digest(credential.token.get_secret_value()),
            )
            for credential in registry.devices
        )

    @property
    def configured_device_count(self) -> int:
        return len(self._credentials)

    def authenticate(self, authorization: str) -> DeviceId | None:
        prefix = "Bearer "
        if not authorization.startswith(prefix):
            return None
        candidate_digest = _token_digest(authorization.removeprefix(prefix))
        matched_device: DeviceId | None = None
        match_count = 0
        for credential in self._credentials:
            matches = hmac.compare_digest(candidate_digest, credential.digest)
            match_count += int(matches)
            if matches:
                matched_device = credential.device_id
        return matched_device if match_count == 1 else None


def _token_digest(token: str) -> bytes:
    return hashlib.sha256(token.encode("utf-8")).digest()
