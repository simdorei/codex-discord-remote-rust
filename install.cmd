@echo off
setlocal
cd /d "%~dp0"
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0install.ps1" -Prebuilt %*
set "INSTALL_RESULT=%ERRORLEVEL%"
echo.
if not "%INSTALL_RESULT%"=="0" echo Installation failed. Read the error above.
if "%INSTALL_RESULT%"=="0" echo Installation complete. The bot has not been started. See docs\windows-release.md for setup.
pause
exit /b %INSTALL_RESULT%
