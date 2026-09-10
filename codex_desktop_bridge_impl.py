from __future__ import annotations

import sys
from types import ModuleType
from typing import Protocol, cast

import codex_desktop_bridge_impl_common as _common
import codex_desktop_bridge_impl_chunk01 as _chunk01
import codex_desktop_bridge_impl_chunk02 as _chunk02
import codex_desktop_bridge_impl_chunk03 as _chunk03
import codex_desktop_bridge_impl_chunk04 as _chunk04
import codex_desktop_bridge_impl_chunk05 as _chunk05
import codex_desktop_bridge_impl_chunk06 as _chunk06
import codex_desktop_bridge_impl_chunk07 as _chunk07


class BridgeAttributeValue(Protocol):
    pass


class BridgeExportContractError(RuntimeError):
    pass


class BridgeExportCollisionError(BridgeExportContractError):
    pass


BRIDGE_MODULES: tuple[ModuleType, ...] = (
    _common,
    _chunk01,
    _chunk02,
    _chunk03,
    _chunk04,
    _chunk05,
    _chunk06,
    _chunk07,
)
IGNORED_DECLARED_EXPORTS = frozenset({"annotations"})


def _declared_export_names(module: ModuleType) -> tuple[str, ...]:
    declared = cast(object, getattr(module, "__all__", None))
    if not isinstance(declared, tuple) or not all(isinstance(name, str) for name in declared):
        raise BridgeExportContractError(
            f"Bridge module {module.__name__} must declare a literal tuple __all__."
        )
    return cast(tuple[str, ...], declared)


def compose_bridge_exports(
    modules: tuple[ModuleType, ...],
) -> dict[str, BridgeAttributeValue]:
    exported: dict[str, BridgeAttributeValue] = {}
    owners: dict[str, str] = {}
    for module in modules:
        for name in _declared_export_names(module):
            if name in IGNORED_DECLARED_EXPORTS:
                continue
            previous_owner = owners.get(name)
            if previous_owner is not None:
                raise BridgeExportCollisionError(
                    f"Bridge export {name!r} is owned by both {previous_owner} and {module.__name__}."
                )
            if not hasattr(module, name):
                raise BridgeExportContractError(
                    f"Bridge module {module.__name__} declares missing export {name!r}."
                )
            owners[name] = module.__name__
            exported[name] = cast(BridgeAttributeValue, getattr(module, name))
    return exported


def _install_bridge_exports(exported: dict[str, BridgeAttributeValue]) -> None:
    globals().update(exported)
    for module in BRIDGE_MODULES:
        module.__dict__.update(exported)


def set_facade_attribute(name: str, value: BridgeAttributeValue) -> None:
    globals()[name] = value
    for module in BRIDGE_MODULES:
        module.__dict__[name] = value


def main() -> int:
    return _chunk06.main()


_install_bridge_exports(compose_bridge_exports(BRIDGE_MODULES))

if __name__ == "__main__":
    sys.exit(main())
