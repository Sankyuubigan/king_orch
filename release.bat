@echo off
cd /d "%~dp0"
echo ========================================
echo   King Orch - Release Build ^& Publish
echo ========================================
node release.cjs
if %ERRORLEVEL% NEQ 0 pause