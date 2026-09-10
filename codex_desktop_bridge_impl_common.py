from __future__ import annotations
import sqlite3

import argparse

from contextlib import nullcontext

import os

import platform

import shutil

import subprocess

import sys

import threading

import time

import uuid

from pathlib import Path

from typing import Callable

import codex_desktop_bridge_cli as bridge_cli

import codex_desktop_bridge_active_thread as active_thread

import codex_desktop_bridge_approval_report as approval_report

import codex_desktop_bridge_archive_commands as archive_commands

import codex_desktop_bridge_archive_delete as archive_delete

import codex_desktop_bridge_archive_retry as archive_retry

import codex_desktop_bridge_background_watch as background_watch

import codex_desktop_bridge_busy_state as busy_state

import codex_desktop_bridge_command_ask as command_ask_runner

import codex_desktop_bridge_command_ask_types as command_ask_types

import codex_desktop_bridge_desktop_commands as desktop_commands

import codex_desktop_bridge_desktop_process as desktop_process

import codex_desktop_bridge_desktop_resolver as desktop_resolver

import codex_desktop_bridge_doctor_report as doctor_report

import codex_desktop_bridge_final_answer as final_answer_watch

import codex_desktop_bridge_file_backup as file_backup

import codex_desktop_bridge_formatting as bridge_formatting

import codex_desktop_bridge_ipc_pipe as ipc_pipe

import codex_desktop_bridge_ipc_runtime as ipc_runtime

import codex_desktop_bridge_ipc_turn as ipc_turn

import codex_desktop_bridge_interactive_session as interactive_session

import codex_desktop_bridge_new_thread as new_thread

import codex_desktop_bridge_new_command as new_command

import codex_desktop_bridge_open_command as open_command

import codex_desktop_bridge_stop_command as stop_command

import codex_desktop_bridge_pending as bridge_pending

import codex_desktop_bridge_permission_ui as permission_ui

import codex_desktop_bridge_prompt_delivery as prompt_delivery

import codex_desktop_bridge_prompt_sender as prompt_sender

import codex_desktop_bridge_protocol as bridge_protocol

import codex_desktop_bridge_reply as bridge_reply

import codex_desktop_bridge_reply_payload as reply_payload

import codex_desktop_bridge_repl as bridge_repl

import codex_desktop_bridge_session_index as session_index

import codex_desktop_bridge_session_sync as session_sync

import codex_desktop_bridge_settings_commands as settings_commands

import codex_desktop_bridge_session_tail as session_tail

import codex_desktop_bridge_sidebar_activation as sidebar_activation

import codex_desktop_bridge_sidecar as sidecar_transport

import codex_desktop_bridge_sidecar_thread as sidecar_thread

import codex_desktop_bridge_sqlite as bridge_sqlite

import codex_desktop_bridge_state as bridge_state

import codex_desktop_bridge_status_report as status_report

import codex_desktop_bridge_tail as bridge_tail

import codex_desktop_bridge_thread_context as thread_context

import codex_desktop_bridge_thread_records as thread_records

import codex_desktop_bridge_thread_actions as thread_actions

import codex_desktop_bridge_thread_list as thread_list

import codex_desktop_bridge_thread_store as thread_store

import codex_desktop_bridge_thread_activation as thread_activation

import codex_desktop_bridge_use_report as use_report

import codex_desktop_bridge_window_focus as window_focus

if os.name == "nt":
    import codex_desktop_bridge_windows_input as windows_input
elif sys.platform == "darwin":
    import codex_desktop_bridge_macos_input as windows_input
else:
    import codex_desktop_bridge_unsupported_input as windows_input

from codex_bridge_state import JsonObject, JsonValue

from codex_session_events import iter_session_events

from codex_desktop_bridge_sidecar import CodexAppServerSidecar, CodexSidecarError as CodexSidecarError

from codex_model_catalog import format_settings_options

from codex_thread_context import coerce_nonnegative_int as coerce_nonnegative_int

from codex_thread_models import ThreadInfo, WindowInfo

from codex_thread_settings import (
    build_thread_settings_update,
)

_arg_text = bridge_cli.arg_text

_arg_optional_text = bridge_cli.arg_optional_text

_arg_bool = bridge_cli.arg_bool

_arg_int = bridge_cli.arg_int

_arg_float = bridge_cli.arg_float

get_env_path = bridge_state.get_env_path

resolve_state_db_path = bridge_state.resolve_state_db_path

get_optional_env_file_path = bridge_state.get_optional_env_file_path

get_float_env = bridge_state.get_float_env

SCRIPT_DIR = bridge_state.SCRIPT_DIR

BRIDGE_ENV_PATH = bridge_state.BRIDGE_ENV_PATH

CODEX_HOME = bridge_state.CODEX_HOME

GLOBAL_STATE_PATH = bridge_state.GLOBAL_STATE_PATH

STATE_DB_PATH = bridge_state.STATE_DB_PATH

SESSION_INDEX_PATH = bridge_state.SESSION_INDEX_PATH

BRIDGE_STATE_PATH = bridge_state.BRIDGE_STATE_PATH

LOG_DB_PATH = bridge_state.LOG_DB_PATH

ARCHIVED_SESSIONS_DIR = bridge_state.ARCHIVED_SESSIONS_DIR

MAINTENANCE_BACKUP_ROOT = bridge_state.MAINTENANCE_BACKUP_ROOT

CODEX_IPC_PIPE = ipc_pipe.CODEX_IPC_PIPE

CODEX_APP_SERVER_EXE_ENV = sidecar_transport.CODEX_APP_SERVER_EXE_ENV

CODEX_DESKTOP_EXE_ENV = "CODEX_DESKTOP_EXE"

CODEX_APP_SERVER_EXE = sidecar_transport.CODEX_APP_SERVER_EXE

SINGLE_BACKUP_LOG_LIMIT_BYTES = file_backup.SINGLE_BACKUP_LOG_LIMIT_BYTES

IPC_PROBE_LOG_PATH = SCRIPT_DIR / "_ipc_probe_log.jsonl"

HIGH_CONTEXT_INPUT_RATIO_THRESHOLD = thread_context.HIGH_CONTEXT_INPUT_RATIO_THRESHOLD

CRITICAL_CONTEXT_INPUT_RATIO_THRESHOLD = thread_context.CRITICAL_CONTEXT_INPUT_RATIO_THRESHOLD

ARCHIVE_RECOMMEND_TOKENS_USED_THRESHOLD = thread_context.ARCHIVE_RECOMMEND_TOKENS_USED_THRESHOLD

ARCHIVE_RECOMMEND_CONTEXT_TOKENS_THRESHOLD = thread_context.ARCHIVE_RECOMMEND_CONTEXT_TOKENS_THRESHOLD

LIVE_APPROVAL_CACHE_MAX_AGE_SEC = bridge_state.LIVE_APPROVAL_CACHE_MAX_AGE_SEC

BACKGROUND_WATCHERS = background_watch.BACKGROUND_WATCHERS

BACKGROUND_WATCHERS_LOCK = background_watch.BACKGROUND_WATCHERS_LOCK

PRINT_LOCK = threading.Lock()

kernel32 = ipc_pipe.kernel32

PROCESS_QUERY_LIMITED_INFORMATION = 0x1000

SW_RESTORE = 9

PIPE_PEEK_RETRY_SEC = ipc_pipe.PIPE_PEEK_RETRY_SEC

VK_CONTROL = 0x11

VK_MENU = 0x12

VK_RETURN = 0x0D

VK_SHIFT = 0x10

VK_BACK = 0x08

VK_TAB = 0x09

VK_ESCAPE = 0x1B

VK_A = 0x41

VK_C = 0x43

VK_J = 0x4A

VK_L = 0x4C

VK_V = 0x56

class SessionFileMissingError(RuntimeError):
    def __init__(self, session_path: Path) -> None:
        super().__init__(f"Session file not found: {session_path}")

__all__ = ('annotations', 'sqlite3', 'argparse', 'nullcontext', 'os', 'platform', 'shutil', 'subprocess', 'sys', 'threading', 'time', 'uuid', 'Path', 'Callable', 'bridge_cli', 'active_thread', 'approval_report', 'archive_commands', 'archive_delete', 'archive_retry', 'background_watch', 'busy_state', 'command_ask_runner', 'command_ask_types', 'desktop_commands', 'desktop_process', 'desktop_resolver', 'doctor_report', 'final_answer_watch', 'file_backup', 'bridge_formatting', 'ipc_pipe', 'ipc_runtime', 'ipc_turn', 'interactive_session', 'new_thread', 'new_command', 'open_command', 'stop_command', 'bridge_pending', 'permission_ui', 'prompt_delivery', 'prompt_sender', 'bridge_protocol', 'bridge_reply', 'reply_payload', 'bridge_repl', 'session_index', 'session_sync', 'settings_commands', 'session_tail', 'sidebar_activation', 'sidecar_transport', 'sidecar_thread', 'bridge_sqlite', 'bridge_state', 'status_report', 'bridge_tail', 'thread_context', 'thread_records', 'thread_actions', 'thread_list', 'thread_store', 'thread_activation', 'use_report', 'window_focus', 'windows_input', 'JsonObject', 'JsonValue', 'iter_session_events', 'CodexAppServerSidecar', 'CodexSidecarError', 'format_settings_options', 'coerce_nonnegative_int', 'ThreadInfo', 'WindowInfo', 'build_thread_settings_update', '_arg_text', '_arg_optional_text', '_arg_bool', '_arg_int', '_arg_float', 'get_env_path', 'resolve_state_db_path', 'get_optional_env_file_path', 'get_float_env', 'SCRIPT_DIR', 'BRIDGE_ENV_PATH', 'CODEX_HOME', 'GLOBAL_STATE_PATH', 'STATE_DB_PATH', 'SESSION_INDEX_PATH', 'BRIDGE_STATE_PATH', 'LOG_DB_PATH', 'ARCHIVED_SESSIONS_DIR', 'MAINTENANCE_BACKUP_ROOT', 'CODEX_IPC_PIPE', 'CODEX_APP_SERVER_EXE_ENV', 'CODEX_DESKTOP_EXE_ENV', 'CODEX_APP_SERVER_EXE', 'SINGLE_BACKUP_LOG_LIMIT_BYTES', 'IPC_PROBE_LOG_PATH', 'HIGH_CONTEXT_INPUT_RATIO_THRESHOLD', 'CRITICAL_CONTEXT_INPUT_RATIO_THRESHOLD', 'ARCHIVE_RECOMMEND_TOKENS_USED_THRESHOLD', 'ARCHIVE_RECOMMEND_CONTEXT_TOKENS_THRESHOLD', 'LIVE_APPROVAL_CACHE_MAX_AGE_SEC', 'BACKGROUND_WATCHERS', 'BACKGROUND_WATCHERS_LOCK', 'PRINT_LOCK', 'kernel32', 'PROCESS_QUERY_LIMITED_INFORMATION', 'SW_RESTORE', 'PIPE_PEEK_RETRY_SEC', 'VK_CONTROL', 'VK_MENU', 'VK_RETURN', 'VK_SHIFT', 'VK_BACK', 'VK_TAB', 'VK_ESCAPE', 'VK_A', 'VK_C', 'VK_J', 'VK_L', 'VK_V', 'SessionFileMissingError')
