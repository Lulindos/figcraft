@echo off
rem Instala o LegatIA Design para esta pessoa (sem administrador). Veja LEIA-ME.txt.
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0install.ps1" %*
echo.
pause
