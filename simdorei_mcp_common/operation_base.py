from __future__ import annotations

from typing import ClassVar

from pydantic import BaseModel, ConfigDict


class OperationRequest(BaseModel):
    """Immutable input for one thread-bound project capability."""

    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True, extra="forbid")


class OperationOutput(BaseModel):
    """Immutable output from one local project capability."""

    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True, extra="forbid")
