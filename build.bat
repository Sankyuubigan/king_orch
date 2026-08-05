@echo off
cd /d "%~dp0"

:: Kill old instance to avoid file lock during build (DLL llama.cpp заняты процессом)
taskkill /f /im king_orch.exe 2>nul

:: Автоопределение и инициализация MSVC (Visual Studio Build Tools)
for /f "usebackq delims=" %%i in (`"%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe" -latest -products * -legacy -property installationPath 2^>nul`) do (
    if exist "%%i\VC\Auxiliary\Build\vcvarsall.bat" (
        call "%%i\VC\Auxiliary\Build\vcvarsall.bat" x64 >nul 2>&1
    )
)

node build.cjs
if %ERRORLEVEL% NEQ 0 (
    echo.
    echo ========================================
    echo   ОШИБКА СБОРКИ! Нажмите любую клавишу...
    echo ========================================
    pause >nul
    exit /b 1
)
