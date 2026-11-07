@echo off
rem Liga as IAs deste computador (Claude, Cursor...) ao LegatIA Design. Veja LEIA-ME.txt.
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0connect-ai.ps1" %*
echo.
pause
