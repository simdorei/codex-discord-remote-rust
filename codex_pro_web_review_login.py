from __future__ import annotations
from collections.abc import Callable, Mapping
from dataclasses import dataclass
import importlib.util
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.request
from typing import Final, Literal
from codex_pro_web_review import DEFAULT_CDP_URL
CHATGPT_URL: Final = "https://chatgpt.com/"
INPUT_SELECTORS: Final = ("#prompt-textarea", 'div[contenteditable="true"]')
FILE_INPUT_SELECTOR: Final = 'input[type="file"]'
LOGIN_WALL_SELECTORS: Final = (
    'button[data-testid="login-button"]',
    'a[href*="auth/login"]',
    'button:has-text("Log in")',
)
BrowserName = Literal["chrome", "comet"]
BrowserState = Literal["ok", "wrong", "down"]
DependencyState = Literal["ok", "missing"]
LoginState = Literal["ok", "no", "unknown"]
@dataclass(frozen=True, slots=True)
class EnvIssue:
    name: str
    hint: str
@dataclass(frozen=True, slots=True)
class ProWebReviewEnvReport:
    deps: DependencyState
    browser: BrowserState
    login: LoginState
    ok: tuple[str, ...]
    issues: tuple[EnvIssue, ...]
def cdp_port() -> int:
    raw = os.environ.get("PRO_WEB_REVIEW_CDP_PORT") or os.environ.get("INSANE_REVIEW_CDP_PORT") or "9222"
    try:
        return int(raw)
    except ValueError:
        return 9222
def cdp_url(port: int | None = None) -> str:
    actual_port = port if port is not None else cdp_port()
    return f"http://127.0.0.1:{actual_port}"
def is_port_open(port: int | None = None) -> bool:
    actual_port = port if port is not None else cdp_port()
    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    try:
        return sock.connect_ex(("127.0.0.1", actual_port)) == 0
    finally:
        sock.close()
def cdp_browser_ok(url: str | None = None) -> bool:
    actual_url = url or cdp_url()
    try:
        with urllib.request.urlopen(f"{actual_url}/json/version", timeout=4) as response:
            info = json.loads(response.read().decode("utf-8"))
    except (OSError, TimeoutError, UnicodeDecodeError, json.JSONDecodeError, urllib.error.URLError):
        return False
    if not isinstance(info, dict):
        return False
    return browser_info_is_cdp_browser(info)
def browser_info_is_cdp_browser(info: Mapping[str, str]) -> bool:
    browser = str(info.get("Browser", ""))
    return any(name in browser for name in ("Chrome", "Chromium", "Comet", "HeadlessChrome", "Edg"))
def ensure_browser(
    browser: BrowserName = "chrome",
    *,
    emit: Callable[[str], None] = print,
    port: int | None = None,
) -> bool:
    actual_port = port if port is not None else cdp_port()
    actual_url = cdp_url(actual_port)
    if is_port_open(actual_port):
        if cdp_browser_ok(actual_url):
            emit(f"CDP browser confirmed on port {actual_port}.")
            return True
        emit(f"Port {actual_port} is open, but it is not a CDP browser.")
        return False
    path = browser_executable(browser)
    if path is None:
        emit(f"{browser} executable was not found. Set PRO_WEB_REVIEW_{browser.upper()} to the browser path.")
        return False
    profile_dir = Path(os.environ.get("PRO_WEB_REVIEW_PROFILE_DIR", str(Path(os.environ.get("LOCALAPPDATA", ".")) / "CodexProWebReviewChrome"))).expanduser()
    emit(f"Starting {browser} with --remote-debugging-port={actual_port}; profile={profile_dir}.")
    try:
        subprocess.Popen(
            [str(path), f"--remote-debugging-port={actual_port}", f"--user-data-dir={profile_dir}", CHATGPT_URL],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
    except OSError as exc:
        emit(f"Failed to start {browser}: {exc}")
        return False
    for _ in range(30):
        if is_port_open(actual_port) and cdp_browser_ok(actual_url):
            time.sleep(2)
            return True
        time.sleep(1)
    emit("Browser start timed out.")
    return False
def browser_executable(browser: BrowserName) -> Path | None:
    env_name = f"PRO_WEB_REVIEW_{browser.upper()}"
    legacy_name = f"INSANE_REVIEW_{browser.upper()}"
    configured = os.environ.get(env_name) or os.environ.get(legacy_name)
    if configured:
        return Path(configured).expanduser()
    for candidate in _browser_candidates(browser):
        if candidate.exists():
            return candidate
    return None
def probe_login(url: str | None = None) -> LoginState:
    actual_url = url or cdp_url()
    if not cdp_browser_ok(actual_url):
        return "unknown"
    if importlib.util.find_spec("playwright") is None:
        return "unknown"
    try:
        from playwright.sync_api import Error as PlaywrightError
        from playwright.sync_api import sync_playwright
    except ImportError:
        return "unknown"
    try:
        with sync_playwright() as playwright:
            browser = playwright.chromium.connect_over_cdp(actual_url)
            context = pick_context(browser)
            if context is None:
                return "no"
            page = context.new_page()
            try:
                page.goto(CHATGPT_URL, wait_until="load", timeout=30_000)
                time.sleep(2)
                return "ok" if looks_logged_in(page) else "no"
            finally:
                page.close()
    except PlaywrightError:
        return "unknown"
def build_env_report(*, do_install: bool = False) -> ProWebReviewEnvReport:
    ok: list[str] = []
    issues: list[EnvIssue] = []
    if do_install:
        _install_missing_python_deps()
    deps = _dependency_state(ok, issues)
    browser = _browser_state(ok, issues)
    login: LoginState = "unknown"
    if deps == "ok" and browser == "ok":
        login = probe_login()
        if login == "ok":
            ok.append("ChatGPT browser login confirmed.")
        elif login == "no":
            issues.append(EnvIssue("ChatGPT login missing", "Open chatgpt.com in the CDP browser and sign in."))
    return ProWebReviewEnvReport(deps=deps, browser=browser, login=login, ok=tuple(ok), issues=tuple(issues))
def check_env(*, do_install: bool = False, emit: Callable[[str], None] = print) -> int:
    report = build_env_report(do_install=do_install)
    emit(format_env_report(report))
    return len(report.issues)
def format_env_report(report: ProWebReviewEnvReport) -> str:
    lines = ["=== pro-web-review environment check ==="]
    lines.extend(f"  OK: {item}" for item in report.ok)
    lines.extend(f"  MISSING: {issue.name}\n      hint: {issue.hint}" for issue in report.issues)
    lines.append(f"STATUS deps={report.deps} browser={report.browser} login={report.login}")
    return "\n".join(lines)
def find_input(page):
    for selector in INPUT_SELECTORS:
        handle = page.query_selector(selector)
        if handle is not None:
            return handle
    return None
def pick_context(browser):
    if not browser.contexts:
        return None
    for context in browser.contexts:
        if _has_auth_cookie(context):
            return context
    for context in browser.contexts:
        if _has_chatgpt_cookie(context):
            return context
    return browser.contexts[0]
def looks_logged_in(page) -> bool:
    if find_input(page) is None:
        return False
    for selector in LOGIN_WALL_SELECTORS:
        if page.query_selector(selector) is not None:
            return False
    for _ in range(6):
        if page.query_selector("button.__composer-pill") is not None:
            return True
        if page.query_selector(FILE_INPUT_SELECTOR) is not None:
            return True
        time.sleep(0.5)
    return False
def _browser_candidates(browser: BrowserName) -> tuple[Path, ...]:
    local = Path(os.environ.get("LOCALAPPDATA", ""))
    program_files = Path(os.environ.get("ProgramFiles", "C:/Program Files"))
    program_files_x86 = Path(os.environ.get("ProgramFiles(x86)", "C:/Program Files (x86)"))
    if browser == "comet":
        return (
            local / "Comet" / "Application" / "comet.exe",
            program_files / "Comet" / "Application" / "comet.exe",
        )
    return (
        program_files / "Google" / "Chrome" / "Application" / "chrome.exe",
        program_files_x86 / "Google" / "Chrome" / "Application" / "chrome.exe",
        local / "Google" / "Chrome" / "Application" / "chrome.exe",
    )
def _dependency_state(ok: list[str], issues: list[EnvIssue]) -> DependencyState:
    missing = []
    for module in ("playwright",):
        if importlib.util.find_spec(module) is None:
            missing.append(module)
        else:
            ok.append(f"python {module} installed.")
    if missing:
        issues.append(EnvIssue("python dependencies missing", f"Install with: {sys.executable} -m pip install {' '.join(missing)}"))
        return "missing"
    return "ok"
def _browser_state(ok: list[str], issues: list[EnvIssue]) -> BrowserState:
    port = cdp_port()
    url = cdp_url(port)
    if is_port_open(port) and cdp_browser_ok(url):
        ok.append(f"CDP browser confirmed on port {port}.")
        return "ok"
    if is_port_open(port):
        issues.append(EnvIssue(f"port {port} is not a CDP browser", "Close the conflicting process or choose another port."))
        return "wrong"
    issues.append(EnvIssue(f"browser CDP port {port} is closed", f"Start Chrome/Comet with --remote-debugging-port={port}."))
    return "down"
def _install_missing_python_deps() -> None:
    for module in ("playwright",):
        if importlib.util.find_spec(module) is None:
            subprocess.run([sys.executable, "-m", "pip", "install", module], check=False)
def _has_auth_cookie(context) -> bool:
    return any(str(cookie.get("name", "")).startswith("__Secure-next-auth") for cookie in _cookies(context))
def _has_chatgpt_cookie(context) -> bool:
    return bool(_cookies(context))
def _cookies(context) -> list[Mapping[str, str]]:
    try:
        cookies = context.cookies(CHATGPT_URL)
    except (AttributeError, TypeError, RuntimeError):
        return []
    if not isinstance(cookies, list):
        return []
    return [cookie for cookie in cookies if isinstance(cookie, dict)]
