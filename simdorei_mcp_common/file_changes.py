from __future__ import annotations

from typing import ClassVar, Literal, Self

from pydantic import BaseModel, ConfigDict, Field, model_validator

from simdorei_mcp_common.operation_base import OperationRequest


class FileChange(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="forbid", frozen=True)

    action: Literal["create", "update", "delete", "move"]
    path: str = Field(min_length=1, max_length=1_000)
    content: str | None = Field(default=None, max_length=1_048_576)
    destination: str | None = Field(default=None, min_length=1, max_length=1_000)
    expected_sha256: str | None = Field(
        default=None,
        pattern=r"^[0-9a-f]{64}$",
    )

    @model_validator(mode="after")
    def validate_action_fields(self) -> Self:
        if self.action in {"create", "update"} and self.content is None:
            raise ValueError(f"{self.action} requires content")
        if (
            self.action in {"update", "delete", "move"}
            and self.expected_sha256 is None
        ):
            raise ValueError(f"{self.action} requires expected_sha256")
        if self.action == "move" and self.destination is None:
            raise ValueError("move requires destination")
        if self.action != "move" and self.destination is not None:
            raise ValueError("destination is only valid for move")
        if self.action == "create" and self.expected_sha256 is not None:
            raise ValueError("create does not accept expected_sha256")
        if self.action == "delete" and self.content is not None:
            raise ValueError("delete does not accept content")
        return self


class FileApplyPatchRequest(OperationRequest):
    kind: Literal["file_apply_patch"] = "file_apply_patch"
    changes: tuple[FileChange, ...] = Field(min_length=1, max_length=200)

    @model_validator(mode="after")
    def bound_total_content(self) -> Self:
        content_size = sum(
            len(change.content.encode("utf-8"))
            for change in self.changes
            if change.content is not None
        )
        if content_size > 10_485_760:
            raise ValueError("combined file change content exceeds 10 MiB")
        return self
