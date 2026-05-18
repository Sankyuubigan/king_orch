@echo off
cd /d "%~dp0"
node build.cjs
if %ERRORLEVEL% NEQ 0 pause