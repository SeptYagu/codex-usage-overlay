@echo off
setlocal
powershell.exe -NoProfile -WindowStyle Hidden -File "%~dp0Start-CodexPetUsage.ps1"
if errorlevel 1 (
  echo.
  echo Could not start Codex Usage Overlay.
  pause
)
