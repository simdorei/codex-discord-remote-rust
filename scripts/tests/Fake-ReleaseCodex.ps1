[CmdletBinding()]
param([Parameter(Mandatory)][string]$OutputPath)
$ErrorActionPreference = 'Stop'
# Build outside the restricted installation child, using Windows PowerShell's
# framework compiler. The fixture is a real executable, never a script launcher.
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.IO;
using System.Text;
using System.Web.Script.Serialization;

public static class ReleaseCodexFixture {
    public static int Main(string[] args) {
        Console.OutputEncoding = new UTF8Encoding(false);
        if (args.Length == 1 && args[0] == "--version") {
            Console.WriteLine("codex-cli 0.166.0-alpha.6");
            return 0;
        }
        if (args.Length >= 2 && args[0] == "plugin") {
            if (args.Length == 4 && args[1] == "marketplace" && args[2] == "add") {
                Console.WriteLine("Registered");
                return 0;
            }
            if (args.Length == 3 && args[1] == "add" &&
                args[2] == "codex-discord-remote@codex-discord-remote") {
                Console.WriteLine("Installed");
                return 0;
            }
            var root = Environment.GetEnvironmentVariable("CDR_RELEASE_TEST_ROOT");
            var json = new JavaScriptSerializer();
            if (args.Length == 4 && args[1] == "marketplace" &&
                args[2] == "list" && args[3] == "--json") {
                Console.WriteLine(json.Serialize(new {
                    marketplaces = new[] { new { name = "codex-discord-remote", root = root } }
                }));
                return 0;
            }
            if (args.Length == 3 && args[1] == "list" && args[2] == "--json") {
                var manifest = json.Deserialize<Dictionary<string, object>>(File.ReadAllText(
                    Path.Combine(root, "plugins", "codex-discord-remote", ".codex-plugin", "plugin.json")));
                Console.WriteLine(json.Serialize(new {
                    installed = new[] { new {
                        pluginId = "codex-discord-remote@codex-discord-remote",
                        version = manifest["version"], installed = true, enabled = true
                    } }
                }));
                return 0;
            }
        }
        Console.Error.WriteLine("Unexpected Codex command during installation");
        return 1;
    }
}
'@ -ReferencedAssemblies System.Web.Extensions -OutputAssembly $OutputPath -OutputType ConsoleApplication
