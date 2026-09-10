from __future__ import annotations

from typing import Annotated, Literal, Self

from pydantic import Field, HttpUrl, model_validator

from simdorei_mcp_common.file_changes import (
    FileApplyPatchRequest as FileApplyPatchRequest,
    FileChange as FileChange,
)
from simdorei_mcp_common.operation_base import OperationRequest
from simdorei_mcp_common.terminal_window_interaction_protocol import (
    TerminalWindowInteractionRequest,
)
from simdorei_mcp_common.terminal_window_protocol import TerminalOperationRequest


class ProjectRulesRequest(OperationRequest):
    kind: Literal["project_rules"] = "project_rules"


class ProjectStatusRequest(OperationRequest):
    kind: Literal["project_status"] = "project_status"


class CodeSearchRequest(OperationRequest):
    kind: Literal["code_search"] = "code_search"
    query: str = Field(min_length=1, max_length=500)
    max_results: int = Field(default=100, ge=1, le=200)


class FileCreateRequest(OperationRequest):
    kind: Literal["file_create"] = "file_create"
    path: str = Field(min_length=1, max_length=1_000)
    content: str = Field(max_length=1_048_576)
    overwrite: bool = False


class CommandListRequest(OperationRequest):
    kind: Literal["command_list"] = "command_list"


class CommandRunRequest(OperationRequest):
    kind: Literal["command_run"] = "command_run"
    command_id: str = Field(min_length=1, max_length=300)
    timeout_seconds: int = Field(default=60, ge=1, le=300)


class RepoStatusRequest(OperationRequest):
    kind: Literal["repo_status"] = "repo_status"


class RepoDiffRequest(OperationRequest):
    kind: Literal["repo_diff"] = "repo_diff"


class GitCommitRequest(OperationRequest):
    kind: Literal["git_commit"] = "git_commit"
    message: str = Field(min_length=1, max_length=500)
    paths: tuple[str, ...] = Field(min_length=1, max_length=200)


class GitPushRequest(OperationRequest):
    kind: Literal["git_push"] = "git_push"
    remote: str = Field(default="origin", pattern=r"^[A-Za-z0-9._-]+$")
    branch: str | None = Field(
        default=None,
        max_length=250,
        pattern=r"^[A-Za-z0-9][A-Za-z0-9._/-]*$",
    )


class SaveImageRequest(OperationRequest):
    kind: Literal["save_image"] = "save_image"
    path: str = Field(min_length=1, max_length=1_000)
    data_base64: str = Field(min_length=4, max_length=7_000_000)
    overwrite: bool = False


class SaveImageFromUrlRequest(OperationRequest):
    kind: Literal["save_image_from_url"] = "save_image_from_url"
    path: str = Field(min_length=1, max_length=1_000)
    url: HttpUrl
    overwrite: bool = False


class ListImagesRequest(OperationRequest):
    kind: Literal["list_images"] = "list_images"


class RetrieveImageRequest(OperationRequest):
    kind: Literal["retrieve_image"] = "retrieve_image"
    path: str = Field(min_length=1, max_length=1_000)


class CheckpointListRequest(OperationRequest):
    kind: Literal["checkpoint_list"] = "checkpoint_list"


class CheckpointShowRequest(OperationRequest):
    kind: Literal["checkpoint_show"] = "checkpoint_show"
    checkpoint_id: str = Field(pattern=r"^cp_[a-f0-9]{16}$")


class CheckpointRestoreRequest(OperationRequest):
    kind: Literal["checkpoint_restore"] = "checkpoint_restore"
    checkpoint_id: str = Field(pattern=r"^cp_[a-f0-9]{16}$")


ComputerApp = Literal["chrome", "notepad"]
ComputerMouseButton = Literal["left", "right", "middle"]
ComputerWindowId = Annotated[int, Field(gt=0)]
ComputerObservationId = Annotated[str, Field(min_length=8, max_length=100)]
ComputerCoordinate = Annotated[int, Field(ge=0, le=100_000)]
ComputerClickCount = Annotated[int, Field(ge=1, le=3)]
ComputerScrollDelta = Annotated[int, Field(ge=-10_000, le=10_000)]
ComputerText = Annotated[str, Field(min_length=1, max_length=4_096)]
ComputerClipboardText = Annotated[str, Field(max_length=100_000)]
ComputerKeyList = Annotated[list[str], Field(min_length=1, max_length=4)]


class ComputerRequestValidationError(ValueError):
    """Raised when a computer request would perform no useful action."""


class ComputerListWindowsRequest(OperationRequest):
    kind: Literal["computer_list_windows"] = "computer_list_windows"


class ComputerActivateRequest(OperationRequest):
    kind: Literal["computer_activate"] = "computer_activate"
    window_id: ComputerWindowId


class ComputerLaunchRequest(OperationRequest):
    kind: Literal["computer_launch"] = "computer_launch"
    app: ComputerApp


class ComputerScreenshotRequest(OperationRequest):
    kind: Literal["computer_screenshot"] = "computer_screenshot"
    window_id: ComputerWindowId


class ObservedComputerRequest(OperationRequest):
    window_id: ComputerWindowId
    observation_id: ComputerObservationId


class ComputerClickRequest(ObservedComputerRequest):
    kind: Literal["computer_click"] = "computer_click"
    x: ComputerCoordinate
    y: ComputerCoordinate
    button: ComputerMouseButton = "left"
    click_count: ComputerClickCount = 1


class ComputerDragRequest(ObservedComputerRequest):
    kind: Literal["computer_drag"] = "computer_drag"
    start_x: ComputerCoordinate
    start_y: ComputerCoordinate
    end_x: ComputerCoordinate
    end_y: ComputerCoordinate


class ComputerScrollRequest(ObservedComputerRequest):
    kind: Literal["computer_scroll"] = "computer_scroll"
    x: ComputerCoordinate
    y: ComputerCoordinate
    delta_x: ComputerScrollDelta = 0
    delta_y: ComputerScrollDelta = 0

    @model_validator(mode="after")
    def require_nonzero_delta(self) -> Self:
        if not self.delta_x and not self.delta_y:
            raise ComputerRequestValidationError(
                "A non-zero scroll amount is required."
            )
        return self


class ComputerTypeTextRequest(ObservedComputerRequest):
    kind: Literal["computer_type_text"] = "computer_type_text"
    text: ComputerText


class ComputerPressKeysRequest(ObservedComputerRequest):
    kind: Literal["computer_press_keys"] = "computer_press_keys"
    keys: tuple[str, ...] = Field(min_length=1, max_length=4)


class ComputerCloseRequest(ObservedComputerRequest):
    kind: Literal["computer_close"] = "computer_close"


class ComputerSetClipboardRequest(ObservedComputerRequest):
    kind: Literal["computer_set_clipboard"] = "computer_set_clipboard"
    text: ComputerClipboardText


class ComputerStopRequest(OperationRequest):
    kind: Literal["computer_stop"] = "computer_stop"


ComputerOperation = (
    ComputerListWindowsRequest
    | ComputerActivateRequest
    | ComputerLaunchRequest
    | ComputerScreenshotRequest
    | ComputerClickRequest
    | ComputerDragRequest
    | ComputerScrollRequest
    | ComputerTypeTextRequest
    | ComputerPressKeysRequest
    | ComputerCloseRequest
    | ComputerSetClipboardRequest
    | ComputerStopRequest
)


ProjectOperation = Annotated[
    ProjectRulesRequest
    | ProjectStatusRequest
    | CodeSearchRequest
    | FileApplyPatchRequest
    | FileCreateRequest
    | CommandListRequest
    | CommandRunRequest
    | RepoStatusRequest
    | RepoDiffRequest
    | GitCommitRequest
    | GitPushRequest
    | SaveImageRequest
    | SaveImageFromUrlRequest
    | ListImagesRequest
    | RetrieveImageRequest
    | CheckpointListRequest
    | CheckpointShowRequest
    | CheckpointRestoreRequest
    | ComputerListWindowsRequest
    | ComputerActivateRequest
    | ComputerLaunchRequest
    | ComputerScreenshotRequest
    | ComputerClickRequest
    | ComputerDragRequest
    | ComputerScrollRequest
    | ComputerTypeTextRequest
    | ComputerPressKeysRequest
    | ComputerCloseRequest
    | ComputerSetClipboardRequest
    | ComputerStopRequest
    | TerminalOperationRequest
    | TerminalWindowInteractionRequest,
    Field(discriminator="kind"),
]
