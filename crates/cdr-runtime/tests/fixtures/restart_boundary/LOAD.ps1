$ErrorActionPreference='Stop'
# Readiness uses the same normalized root as the real watchdog entry point.
$RepoRoot=[IO.Path]::GetFullPath($env:V2_ROOT)
function Import-SourceFunctions([string]$Relative) {
 $tokens=$null; $errors=$null
 $source=[IO.File]::ReadAllText((Join-Path $env:V2_SOURCE $Relative),[Text.Encoding]::UTF8)
 $ast=[Management.Automation.Language.Parser]::ParseInput($source,[ref]$tokens,[ref]$errors)
 if($errors.Count){throw $errors[0].Message}
 $ast.FindAll({param($n) $n -is [Management.Automation.Language.FunctionDefinitionAst]},$false) |
  ForEach-Object { $_.Extent.Text }
}
