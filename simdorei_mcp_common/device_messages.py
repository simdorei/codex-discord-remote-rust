from __future__ import annotations

from datetime import datetime
from typing import ClassVar

from pydantic import BaseModel, ConfigDict

from simdorei_mcp_common.messages import DeviceId


class DeviceSummary(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True, extra="forbid")

    device_id: DeviceId
    online: bool


class DeviceListOutput(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True, extra="forbid")

    devices: tuple[DeviceSummary, ...]


class DeviceSelectionOutput(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True, extra="forbid")

    device_id: DeviceId
    working_directory: str
    expires_at: datetime
