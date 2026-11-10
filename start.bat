@echo off
REM Double-click QoL wrapper for start.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0start.ps1" %*
if errorlevel 1 (
  echo.
  echo [deskcraft] exited with error - window held open
  pause
  exit /b 1
)
