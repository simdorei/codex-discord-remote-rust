from __future__ import annotations

from dataclasses import dataclass
from enum import StrEnum, unique

from codex_app_server_transport_replies import CodexAppServerTransportError


@dataclass(frozen=True, slots=True)
class AppServerLifecycleSnapshot:
    generation: int
    healthy: bool
    accepting_since: float | None
    quarantined: bool = False
    restart_pending: bool = False
    read_degraded: bool = False
    consecutive_read_timeouts: int = 0
    plugin_runtime_fingerprint: str | None = None
    plugin_runtime_error: str | None = None


class AppServerGenerationExpiredError(CodexAppServerTransportError):
    pass


class AppServerGenerationQuarantinedError(AppServerGenerationExpiredError):
    generation: int

    def __init__(self, *, generation: int) -> None:
        super().__init__(
            f"Codex app-server generation {generation} is quarantined pending a safe restart."
        )
        self.generation = generation


class AppServerGenerationMismatch(AppServerGenerationExpiredError):
    expected_generation: int
    actual_generation: int
    healthy: bool

    def __init__(
        self,
        *,
        expected_generation: int,
        actual_generation: int,
        healthy: bool,
    ) -> None:
        state = "healthy" if healthy else "unhealthy"
        super().__init__(
            "Codex app-server lifecycle changed before delivery: "
            + f"expected generation {expected_generation}, "
            + f"found generation {actual_generation} ({state})."
        )
        self.expected_generation = expected_generation
        self.actual_generation = actual_generation
        self.healthy = healthy


@unique
class ChildCleanupRecycleStatus(StrEnum):
    RECYCLED = "recycled"
    NO_CLEANUP_DEBT = "no_cleanup_debt"
    RECYCLE_BUSY = "recycle_busy"
    ACTIVE_DELIVERY = "active_delivery"
    ACTIVE_TURN = "active_turn"
    PENDING_REQUEST = "pending_request"
    SUBSCRIBED_THREAD = "subscribed_thread"
    EXTERNAL_WORK = "external_work"


@dataclass(frozen=True, slots=True)
class ChildCleanupRecycleOutcome:
    status: ChildCleanupRecycleStatus
    generation: int

    @property
    def recycled(self) -> bool:
        return self.status is ChildCleanupRecycleStatus.RECYCLED
