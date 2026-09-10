from __future__ import annotations

import sys
from types import ModuleType
from typing import Protocol, override

import codex_desktop_bridge_impl as _IMPL


class FacadeAttributeValue(Protocol):
    pass


def _is_dunder(name: str) -> bool:
    return name.startswith("__") and name.endswith("__")


def _export_names() -> list[str]:
    return [name for name in dir(_IMPL) if not _is_dunder(name)]


class _BridgeFacadeModule(ModuleType):
    @override
    def __setattr__(self, name: str, value: FacadeAttributeValue) -> None:
        ModuleType.__setattr__(self, name, value)
        if not _is_dunder(name):
            _IMPL.set_facade_attribute(name, value)


for _name in _export_names():
    globals()[_name] = getattr(_IMPL, _name)

main = _IMPL.main

sys.modules[__name__].__class__ = _BridgeFacadeModule

if __name__ == "__main__":
    sys.exit(main())
