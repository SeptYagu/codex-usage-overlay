@echo off
setlocal
chcp 65001 >nul
powershell.exe -NoProfile -WindowStyle Hidden -File "%~dp0Start-CodexPetUsage.ps1"
if errorlevel 1 (
  echo.
  echo Codex 用量悬浮窗启动失败 / Could not start Codex Usage Overlay.
  pause
)
