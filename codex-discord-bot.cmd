@echo off
setlocal

set "SCRIPT_DIR=%~dp0"
set "DISABLE_FILE=%SCRIPT_DIR%.codex_discord_bot.disabled"
set "LAUNCHER_LOG_PATH=%SCRIPT_DIR%discord_launcher.log"
if not exist "%DISABLE_FILE%" goto runtime_selection
echo Codex Discord bot launch is disabled by marker: "%DISABLE_FILE%"
set "LAUNCHER_LOG_MESSAGE=direct_launch_disabled marker=%DISABLE_FILE%"
call :log_launcher
exit /b 0

:runtime_selection
set "SCRIPT_ARGS=%*"
set "ENV_FILE=%SCRIPT_DIR%.env"
set "RUNTIME_MODE=%CODEX_DISCORD_RUNTIME%"
set "RUNTIME_MODE_FILE=%SCRIPT_DIR%.codex_discord_runtime"
if not defined RUNTIME_MODE if exist "%RUNTIME_MODE_FILE%" set /p RUNTIME_MODE=<"%RUNTIME_MODE_FILE%"
if not defined RUNTIME_MODE set "RUNTIME_MODE=rust"
if /I "%RUNTIME_MODE%"=="rust" goto run_rust
echo ERROR: This installation only supports the Rust runtime.
echo Unsupported runtime selection: "%RUNTIME_MODE%".
exit /b 1

:run_rust
set "RUST_BINARY=%SCRIPT_DIR%target\release\cdr-runtime.exe"
if not exist "%RUST_BINARY%" (
  echo ERROR: Rust runtime not found: "%RUST_BINARY%"
  echo Run .\install.ps1 -Runtime rust first.
  exit /b 1
)
echo Codex Discord Remote Rust runtime is starting.
pushd "%SCRIPT_DIR%"
if errorlevel 1 (
  echo ERROR: Could not enter the Codex Discord runtime directory.
  exit /b 1
)
"%RUST_BINARY%" --env "%ENV_FILE%" %SCRIPT_ARGS%
set "EXIT_CODE=%errorlevel%"
popd
exit /b %EXIT_CODE%

:log_launcher
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "$path=$env:LAUNCHER_LOG_PATH; $message=$env:LAUNCHER_LOG_MESSAGE; if ($path -and $message) { Add-Content -LiteralPath $path -Encoding UTF8 -Value ('[' + (Get-Date).ToString('s') + '] ' + $message) }" >nul 2>nul
set "LAUNCHER_LOG_MESSAGE="
exit /b 0
