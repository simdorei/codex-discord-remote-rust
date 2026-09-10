from __future__ import annotations

from dataclasses import dataclass
from typing import override


@dataclass(frozen=True, slots=True)
class BrokerError(Exception):
    reason: str

    @override
    def __str__(self) -> str:
        return self.reason


class ActiveBindingMissingError(BrokerError):
    """Raised when a ChatGPT session has no live project binding."""


class BindingCodeError(BrokerError):
    """Raised when a binding code is unknown or expired."""


class SessionProjectConflictError(BrokerError):
    """Raised when one ChatGPT conversation is reused for another Codex thread."""


class BridgeUnavailableError(BrokerError):
    """Raised when the bound local device is disconnected."""


class BridgeTimeoutError(BrokerError):
    """Raised when the local bridge does not answer in time."""


class RemoteOperationError(BrokerError):
    """Raised when the local filesystem operation is rejected."""


class BridgeProtocolError(BrokerError):
    """Raised when the local bridge returns the wrong result variant."""
