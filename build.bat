@echo off
cd /d "%~dp0"
node build.cjs
if %ERRORLEVEL% NEQ 0 (
    echo.
    echo ========================================
    echo   ОШИБКА СБОРКИ! Нажмите любую клавишу...
    echo ========================================
    pause >nul
    exit /b 1
)