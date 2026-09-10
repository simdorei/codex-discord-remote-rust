from __future__ import annotations

import threading
from dataclasses import dataclass
from typing import Callable, cast, final

from codex_app_server_transport_replies import JsonMapping, JsonObject


LogFunc = Callable[[str], None]


def _mapping(value: object) -> JsonMapping | None:
    return cast(JsonMapping, value) if isinstance(value, dict) else None


def _text(value: object) -> str:
    return str(value or "").strip()


def _normalized_tool(value: object) -> str:
    return "".join(character for character in _text(value).lower() if character.isalnum())


def _thread_ids(value: object) -> list[str]:
    if not isinstance(value, list):
        return []
    items = cast(list[object], value)
    return [thread_id for item in items if (thread_id := _text(item))]


@dataclass(frozen=True, slots=True)
class ChildLifecycleSnapshot:
    generation: int
    cleanup_pending: bool
    root_thread_ids: tuple[str, ...]
    child_thread_ids: tuple[str, ...]


@final
class ChildLifecycleTracker:
    def __init__(self) -> None:
        self._lock = threading.RLock()
        self._generation = 0
        self._parent_by_child: dict[str, str] = {}
        self._cleanup_pending = False

    def reset(self, generation: int) -> None:
        with self._lock:
            self._generation = generation
            self._parent_by_child.clear()
            self._cleanup_pending = False

    def snapshot(self, generation: int) -> ChildLifecycleSnapshot:
        with self._lock:
            if generation != self._generation:
                return ChildLifecycleSnapshot(generation, False, (), ())
            roots = {self._root_for(child_id) for child_id in self._parent_by_child}
            roots.discard("")
            return ChildLifecycleSnapshot(
                generation=generation,
                cleanup_pending=self._cleanup_pending,
                root_thread_ids=tuple(sorted(roots)),
                child_thread_ids=tuple(sorted(self._parent_by_child)),
            )

    def record_notification(
        self,
        message: JsonObject,
        *,
        generation: int,
        log: LogFunc,
    ) -> None:
        params = _mapping(message.get("params"))
        if params is None:
            return
        method = _text(message.get("method"))
        relations = (
            self._thread_started_relations(params)
            if method == "thread/started"
            else self._collaboration_relations(params)
        )
        for child_id, parent_id in relations:
            self._record_relation(
                child_id,
                parent_id,
                generation=generation,
                log=log,
            )

    def _record_relation(
        self,
        child_id: str,
        parent_id: str,
        *,
        generation: int,
        log: LogFunc,
    ) -> None:
        if not child_id or not parent_id or child_id == parent_id:
            return
        with self._lock:
            if generation != self._generation:
                return
            existing = self._parent_by_child.get(child_id)
            if existing == parent_id:
                return
            self._parent_by_child[child_id] = parent_id
            self._cleanup_pending = True
            root_id = self._root_for(child_id)
        log(
            "app_server_child_observed "
            + f"generation={generation} root={root_id or parent_id} "
            + f"parent={parent_id} child={child_id}"
        )

    def _root_for(self, thread_id: str) -> str:
        current = thread_id
        seen: set[str] = set()
        while current and current not in seen:
            seen.add(current)
            parent = self._parent_by_child.get(current)
            if not parent:
                return current
            current = parent
        return ""

    @staticmethod
    def _thread_started_relations(params: JsonMapping) -> list[tuple[str, str]]:
        thread = _mapping(params.get("thread"))
        if thread is None:
            return []
        child_id = _text(thread.get("id"))
        parent_id = _text(thread.get("parentThreadId") or thread.get("parent_thread_id"))
        if not parent_id:
            source = _mapping(thread.get("source"))
            subagent = _mapping(source.get("subAgent")) if source is not None else None
            spawn = _mapping(subagent.get("thread_spawn")) if subagent is not None else None
            if spawn is not None:
                parent_id = _text(spawn.get("parent_thread_id") or spawn.get("parentThreadId"))
        return [(child_id, parent_id)] if child_id and parent_id else []

    @staticmethod
    def _collaboration_relations(params: JsonMapping) -> list[tuple[str, str]]:
        item = _mapping(params.get("item"))
        if item is None:
            return []
        if _normalized_tool(item.get("type")) != "collabagenttoolcall":
            return []
        if _normalized_tool(item.get("tool")) != "spawnagent":
            return []
        parent_id = _text(item.get("senderThreadId") or params.get("threadId"))
        child_ids = _thread_ids(item.get("receiverThreadIds"))
        if not child_ids:
            states = item.get("agentsStates")
            if isinstance(states, dict):
                child_ids = [
                    _text(thread_id)
                    for thread_id in cast(dict[object, object], states)
                    if _text(thread_id)
                ]
            elif isinstance(states, list):
                child_ids = [
                    child_id
                    for state in cast(list[object], states)
                    if (mapping := _mapping(state)) is not None
                    if (child_id := _text(mapping.get("threadId")))
                ]
        return [(child_id, parent_id) for child_id in child_ids if parent_id and child_id != parent_id]
