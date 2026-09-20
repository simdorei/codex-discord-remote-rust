@echo off
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0codex-discord-rust-restart.ps1" -Force %*
exit /b %errorlevel%
