@echo off
cd /d "%~dp0"

:: Автоопределение и инициализация MSVC (Visual Studio Build Tools)
for /f "usebackq delims=" %%i in (`"%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe" -latest -products * -legacy -property installationPath 2^>nul`) do (
    if exist "%%i\VC\Auxiliary\Build\vcvarsall.bat" (
        call "%%i\VC\Auxiliary\Build\vcvarsall.bat" x64 >nul 2>&1
    )
)

cd src-tauri
cargo test
if %ERRORLEVEL% NEQ 0 (
    echo.
    echo ========================================
    echo      ТЕСТЫ НЕ ПРОШЛИ!
    echo ========================================
    pause >nul
    exit /b 1
)
