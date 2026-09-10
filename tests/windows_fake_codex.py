from __future__ import annotations

import json
import os
import subprocess
import textwrap
from pathlib import Path


def compile_fake_codex_exe(
    repo_root: Path,
    marketplace_json: str,
    plugin_json: str,
    *,
    large_failure_output: bool,
) -> Path:
    framework_root = Path(os.environ.get("WINDIR", r"C:\Windows")) / (
        "Microsoft.NET/Framework"
    )
    compiler = framework_root / "v4.0.30319/csc.exe"
    if not compiler.is_file():
        raise RuntimeError(f".NET Framework C# compiler was not found: {compiler}")

    raw_marketplace_json = json.dumps(
        json.loads(marketplace_json), ensure_ascii=False
    )
    raw_plugin_json = json.dumps(json.loads(plugin_json), ensure_ascii=False)
    large_failure_source = ""
    if large_failure_output:
        large_failure_source = textwrap.dedent(
            """\
            if (StartsWith(arguments, "plugin", "marketplace", "add"))
            {
                Console.Out.Write(new string('O', 131072));
                Console.Out.WriteLine(" STDOUT_TAIL");
                Console.Error.Write(new string('E', 131072));
                Console.Error.WriteLine(" STDERR_TAIL");
                return 7;
            }
            """
        )
    source = repo_root / "fake-codex.cs"
    _ = source.write_text(
        textwrap.dedent(
            f"""\
            using System;
            using System.IO;
            using System.Text;

            internal static class FakeCodex
            {{
                private static readonly string MarketplaceJson = {json.dumps(raw_marketplace_json)};
                private static readonly string PluginJson = {json.dumps(raw_plugin_json)};

                private static bool StartsWith(string[] actual, params string[] expected)
                {{
                    if (actual.Length < expected.Length) return false;
                    for (int index = 0; index < expected.Length; index++)
                    {{
                        if (actual[index] != expected[index]) return false;
                    }}
                    return true;
                }}

                public static int Main(string[] arguments)
                {{
                    Console.OutputEncoding = new UTF8Encoding(false);
                    Console.InputEncoding = new UTF8Encoding(false);
                    {large_failure_source}
                    if (StartsWith(arguments, "plugin", "marketplace", "add"))
                    {{
                        if (arguments.Length < 4 || !File.Exists(Path.Combine(arguments[3], "runtime-release.json")))
                        {{
                            Console.Error.WriteLine("space-path argument mismatch");
                            return 9;
                        }}
                        return 0;
                    }}
                    if (StartsWith(arguments, "plugin", "marketplace", "list"))
                    {{
                        Console.WriteLine(MarketplaceJson);
                        return 0;
                    }}
                    if (StartsWith(arguments, "plugin", "list", "--json"))
                    {{
                        Console.Error.WriteLine("warning: UTF-8 exe inventory");
                        Console.WriteLine(PluginJson);
                        return 0;
                    }}
                    return 0;
                }}
            }}
            """
        ),
        encoding="utf-8",
    )
    executable = repo_root / "fake-codex.exe"
    completed = subprocess.run(
        [
            str(compiler),
            "/nologo",
            "/target:exe",
            f"/out:{executable}",
            str(source),
        ],
        capture_output=True,
        text=True,
        timeout=30,
        check=False,
    )
    if completed.returncode != 0:
        raise RuntimeError(
            "Failed to compile fake Codex executable:\n"
            + completed.stdout
            + completed.stderr
        )
    return executable
