from __future__ import annotations

from collections.abc import Mapping
from types import ModuleType
from weakref import WeakKeyDictionary
import threading


class RuntimeBindingCollisionError(RuntimeError):
    pass


class RuntimeStateBindingError(RuntimeError):
    pass


_BINDING_OWNERS: WeakKeyDictionary[ModuleType, dict[str, str]] = WeakKeyDictionary()
_BINDING_LOCK = threading.RLock()


def install_runtime_bindings(
    module: ModuleType,
    *,
    owner: str,
    values: Mapping[str, object],
) -> None:
    if not owner:
        raise ValueError("Runtime binding owner must not be empty.")

    with _BINDING_LOCK:
        owners = _BINDING_OWNERS.setdefault(module, {})
        collisions = {
            name: previous_owner
            for name in values
            if (previous_owner := owners.get(name)) is not None
            and previous_owner != owner
        }
        if collisions:
            details = ", ".join(
                f"{name} (owned by {previous_owner})"
                for name, previous_owner in sorted(collisions.items())
            )
            raise RuntimeBindingCollisionError(
                f"{owner} cannot replace runtime bindings: {details}."
            )

        for name, value in values.items():
            setattr(module, name, value)
            owners[name] = owner


def update_runtime_state(module: ModuleType, values: Mapping[str, object]) -> None:
    with _BINDING_LOCK:
        missing = sorted(name for name in values if not hasattr(module, name))
        if missing:
            raise RuntimeStateBindingError(
                f"Runtime state must be installed before update: {', '.join(missing)}."
            )
        for name, value in values.items():
            setattr(module, name, value)
