"""Isolated real-turn acceptance probe; never edits installed configuration.

Windows job containment owns only this Python process and descendants. An MCP
approval is held for an explicit reply file; the harness never auto-approves.
"""
import argparse
import ctypes
from ctypes import wintypes
import datetime
import hashlib
import http.server
import json
import os
from pathlib import Path
import queue
import subprocess
import threading
import time


def contain_process_tree():
    class BASIC(ctypes.Structure):
        _fields_ = [("process_time", ctypes.c_longlong), ("job_time", ctypes.c_longlong),
                    ("flags", wintypes.DWORD), ("min_working", ctypes.c_size_t),
                    ("max_working", ctypes.c_size_t), ("active", wintypes.DWORD),
                    ("affinity", ctypes.c_size_t), ("priority", wintypes.DWORD),
                    ("scheduling", wintypes.DWORD)]
    class IO(ctypes.Structure):
        _fields_ = [(name, ctypes.c_ulonglong) for name in
                    ("read_ops", "write_ops", "other_ops", "read_bytes", "write_bytes", "other_bytes")]
    class EXTENDED(ctypes.Structure):
        _fields_ = [("basic", BASIC), ("io", IO), ("process_memory", ctypes.c_size_t),
                    ("job_memory", ctypes.c_size_t), ("peak_process", ctypes.c_size_t),
                    ("peak_job", ctypes.c_size_t)]
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.CreateJobObjectW.argtypes = [ctypes.c_void_p, wintypes.LPCWSTR]
    kernel.CreateJobObjectW.restype = wintypes.HANDLE
    kernel.GetCurrentProcess.restype = wintypes.HANDLE
    kernel.SetInformationJobObject.argtypes = [wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD]
    kernel.AssignProcessToJobObject.argtypes = [wintypes.HANDLE, wintypes.HANDLE]
    handle = kernel.CreateJobObjectW(None, None)
    limits = EXTENDED()
    limits.basic.flags = 0x2000  # JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
    if not handle or not kernel.SetInformationJobObject(handle, 9, ctypes.byref(limits), ctypes.sizeof(limits)):
        raise ctypes.WinError(ctypes.get_last_error())
    if not kernel.AssignProcessToJobObject(handle, kernel.GetCurrentProcess()):
        raise ctypes.WinError(ctypes.get_last_error())
    return handle  # held until this harness process exits


def toml(value):
    if isinstance(value, dict):
        return "{" + ",".join(json.dumps(k) + "=" + toml(v) for k, v in value.items()) + "}"
    if isinstance(value, list):
        return "[" + ",".join(map(toml, value)) + "]"
    return json.dumps(value, ensure_ascii=True)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--prompt", required=True, type=Path)
    parser.add_argument("--local-page", action="store_true")
    parser.add_argument("--runtime", type=Path, help="Use this compiled cdr-runtime MCP entrypoint")
    args = parser.parse_args()
    job_handle = contain_process_tree()
    args.output.mkdir(parents=True, exist_ok=False)
    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    server = manifest["mcpServers"]["cua_repl"]
    cli = server["env"]["CODEX_CLI_PATH"]
    source = Path(__file__).resolve().parent
    host_source = source.parents[1] / "crates/cdr-runtime/src/local_computer/browser_host"
    manifest_digest = hashlib.sha256(args.manifest.read_bytes()).hexdigest()
    if args.runtime:
        if not args.runtime.is_absolute() or not args.runtime.is_file():
            raise ValueError("--runtime must be an absolute candidate executable")
        server = {**server, "command": str(args.runtime), "args": [
            "--official-browser-host", "--manifest", str(args.manifest),
            "--expected-manifest-sha256", manifest_digest,
            "--cache-root", str(args.output / "embedded-host"),
            "--audit-directory", str(args.output),
        ]}
    else:
        server = {**server, "args": [str(host_source / "launcher.mjs"), "--manifest", str(args.manifest),
                                   "--expected-manifest-sha256", manifest_digest,
                                   "--audit-directory", str(args.output)]}
    environment = os.environ.copy()
    for name in list(environment):
        if name.startswith("BROWSER_USE_ACCESS_") or name in {
            "CODEX_APP_TOOLS_PIPE_PATH", "CODEX_TASK_WORKSPACE_VERIFYING_IDENTITY",
            "SKY_CUA_NATIVE_PIPE_DIRECTORY", "CODEX_INTERNAL_ORIGINATOR_OVERRIDE",
            "CODEX_SESSION_ID", "CODEX_THREAD_ID",
        }:
            environment.pop(name, None)
    environment["CODEX_HOME"] = str(Path.home() / ".codex")
    command = [cli, "app-server", "--listen", "stdio://", "-c", "mcp_servers.cua_repl=" + toml(server),
               "-c", 'mcp_servers.cdr_computer={enabled=false,command=' + json.dumps(cli) + ',args=["--version"]}']
    process = subprocess.Popen(command, cwd=source, env=environment, stdin=subprocess.PIPE,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                               text=True, encoding="utf-8", creationflags=subprocess.CREATE_NO_WINDOW)
    events = queue.Queue()
    stderr = []
    receipt = {"started_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
               "source_sha256": {p.name: hashlib.sha256(p.read_bytes()).hexdigest()
                                 for p in host_source.glob("*.mjs")},
               "manifest_sha256": manifest_digest,
               "app_server_pid": process.pid, "configuration_modified": False,
               "approvals_auto_accepted": 0, "gui_stopped_by_harness": False}
    if args.runtime:
        with args.runtime.open("rb") as file:
            receipt["runtime_sha256"] = hashlib.file_digest(file, "sha256").hexdigest()
        receipt["runtime"] = str(args.runtime)
        receipt["runtime_build_info"] = json.loads(subprocess.check_output(
            [str(args.runtime), "--build-info"], text=True, encoding="utf-8",
            creationflags=subprocess.CREATE_NO_WINDOW))
    page_server = None
    test_url = ""
    if args.local_page:
        class TestPage(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_args):
                pass
            def do_GET(self):
                page = b'''<!doctype html><meta charset="utf-8"><title>CDR Browser Host Test</title>
<h1>Standalone browser host acceptance</h1><label>Test text <input id="value"></label>
<button id="apply">Apply test text</button><p id="result">Waiting for test</p>
<script>document.getElementById('apply').onclick=async()=>{
const value=document.getElementById('value').value;
document.getElementById('result').textContent='Applied: '+value;
await fetch('/result',{method:'POST',body:JSON.stringify({value})});};</script>'''
                self.send_response(200)
                self.send_header("Content-Type", "text/html; charset=utf-8")
                self.send_header("Content-Length", str(len(page)))
                self.end_headers()
                self.wfile.write(page)
            def do_POST(self):
                length = int(self.headers.get("Content-Length", "0"))
                if self.path != "/result" or not 0 < length < 4096:
                    self.send_error(400)
                    return
                value = json.loads(self.rfile.read(length))
                with (args.output / "page-results.jsonl").open("a", encoding="utf-8") as file:
                    file.write(json.dumps({"at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
                                           "result": value}) + "\n")
                self.send_response(204)
                self.end_headers()
        page_server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), TestPage)
        threading.Thread(target=page_server.serve_forever, daemon=True).start()
        test_url = "http://127.0.0.1:" + str(page_server.server_port) + "/"
        receipt["test_url"] = test_url
    def read_stdout():
        for line in process.stdout:
            try:
                events.put(json.loads(line))
            except json.JSONDecodeError:
                events.put({"invalid_output": True})
        events.put(None)
    def read_stderr():
        for line in process.stderr:
            if len(stderr) < 5000:
                stderr.append(line)
            if "cdr_browser_host" in line:
                print(line.strip(), flush=True)
    threading.Thread(target=read_stdout, daemon=True).start()
    threading.Thread(target=read_stderr, daemon=True).start()
    def send(message):
        process.stdin.write(json.dumps(message) + "\n")
        process.stdin.flush()
    def observe(message):
        if message is None:
            raise RuntimeError("app-server ended")
        method = message.get("method", "")
        if method in {"item/completed", "turn/completed", "error"}:
            with (args.output / "events.jsonl").open("a", encoding="utf-8") as file:
                file.write(json.dumps(message, ensure_ascii=False) + "\n")
            item = message.get("params", {}).get("item", {})
            print(json.dumps({"event": method, "type": item.get("type"),
                              "tool": item.get("tool"), "text": item.get("text", "")[:1800]}, ensure_ascii=False), flush=True)
        if "id" in message and "method" in message:
            if method != "mcpServer/elicitation/request":
                send({"id": message["id"], "error": {"code": -32601, "message": "Probe does not implement this client method"}})
                return
            pending = args.output / "approval-pending.json"
            pending.write_text(json.dumps(message, ensure_ascii=False, indent=2), encoding="utf-8")
            print(json.dumps({"approval_pending": str(pending), "method": method}, ensure_ascii=False), flush=True)
            deadline = time.monotonic() + 600
            reply_path = args.output / "approval-reply.json"
            while time.monotonic() < deadline:
                if reply_path.exists():
                    reply = json.loads(reply_path.read_text(encoding="utf-8"))
                    if reply.get("id") != message["id"]:
                        raise RuntimeError("approval reply ID does not match")
                    send(reply)
                    reply_path.rename(args.output / ("approval-replied-" + str(message["id"]) + ".json"))
                    return
                time.sleep(0.2)
            raise TimeoutError("approval remained unanswered")
    def request(identifier, method, params):
        send({"id": identifier, "method": method, "params": params})
        deadline = time.monotonic() + 120
        while time.monotonic() < deadline:
            message = events.get(timeout=max(0.1, deadline - time.monotonic()))
            if message and message.get("id") == identifier and "method" not in message:
                if "error" in message:
                    raise RuntimeError(str(message["error"]))
                return message["result"]
            observe(message)
        raise TimeoutError(method)
    try:
        request(1, "initialize", {"clientInfo": {"name": "cdr-browser-host-qa", "version": "1"},
                "capabilities": {"experimentalApi": True, "mcpServerOpenaiFormElicitation": True}})
        send({"method": "initialized", "params": {}})
        started = request(2, "thread/start", {"cwd": str(source), "ephemeral": True,
                          "approvalPolicy": "on-request"})
        thread = started["thread"]["id"]
        receipt.update(thread_id=thread, sandbox=started.get("sandbox"), model=started.get("model"))
        print(json.dumps({"thread_id": thread, "model": receipt["model"]}), flush=True)
        turn = request(3, "turn/start", {"threadId": thread, "input": [
            {"type": "text", "text": args.prompt.read_text(encoding="utf-8").replace("{TEST_URL}", test_url)}
        ]})
        receipt["turn_id"] = turn["turn"]["id"]
        deadline = time.monotonic() + 600
        while time.monotonic() < deadline:
            try:
                message = events.get(timeout=max(0.1, min(30, deadline - time.monotonic())))
            except queue.Empty:
                continue
            observe(message)
            if message and message.get("method") == "turn/completed":
                receipt["turn"] = message["params"]["turn"]
                break
        else:
            raise TimeoutError("model turn")
    except Exception as error:
        receipt["error"] = str(error)
    finally:
        process.stdin.close()
        try:
            process.wait(timeout=8)
        except subprocess.TimeoutExpired:
            process.terminate()
            process.wait(timeout=4)
        receipt["outer_exit_code"] = process.returncode
        if page_server:
            page_server.shutdown()
            page_server.server_close()
            page_results = args.output / "page-results.jsonl"
            observed = [json.loads(line) for line in page_results.read_text(encoding="utf-8").splitlines()] if page_results.exists() else []
            receipt["fixture_click_observed"] = any(
                row.get("result", {}).get("value") == "cdr-standalone-host-ok" for row in observed)
        receipt["finished_at"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        (args.output / "receipt.json").write_text(json.dumps(receipt, ensure_ascii=False, indent=2), encoding="utf-8")
        (args.output / "stderr.log").write_text("".join(stderr), encoding="utf-8")
        print(json.dumps({"receipt": str(args.output / "receipt.json"), "error": receipt.get("error")}), flush=True)
    return 1 if "error" in receipt else 0


if __name__ == "__main__":
    raise SystemExit(main())
