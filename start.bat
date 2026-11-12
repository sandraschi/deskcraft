@echo off
REM Craft app launcher (vendored from mcp-central-docs\templates\craft-app). Double-click or run
REM with the same arguments as start.ps1. The window stays open on errors.
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0start.ps1" %*
if errorlevel 1 (
  echo.
  echo [%~n0] exited with error %errorlevel% - window held open
  pause
  exit /b 1
)
exit /b 0
