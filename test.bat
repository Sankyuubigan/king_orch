@echo off
cd /d "%~dp0"

REM Kill old instance to avoid file lock during build
taskkill /f /im king_orch.exe 2>nul

REM Reset sccache-wrapped compilers (see desktop_rust_tauri/rules.md:98)
set "CC="
set "CXX="
set "CMAKE_C_COMPILER_LAUNCHER="
set "RUSTC_WRAPPER="
set "CARGO_BUILD_RUSTC_WRAPPER="

REM Auto-detect and init MSVC (Visual Studio Build Tools)
for /f "usebackq delims=" %%i in (`"%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe" -latest -products * -legacy -property installationPath 2^>nul`) do (
    if exist "%%i\VC\Auxiliary\Build\vcvarsall.bat" (
        call "%%i\VC\Auxiliary\Build\vcvarsall.bat" x64 >nul 2>&1
    )
)

cd src-tauri

REM Optional test filter: test.bat llm_history
set "FILTER=%~1"

REM NB: у пакета НЕТ lib-таргета (только bin king_orch), поэтому --lib невозможен.
REM cargo test без флага покрывает все таргеты пакета (bin unit-тесты).
if "%FILTER%"=="" (
    cargo test
) else (
    cargo test %FILTER%
)

if %ERRORLEVEL% NEQ 0 (
    echo.
    echo ========================================
    echo      TESTS FAILED!
    echo ========================================
    pause >nul
    exit /b 1
)
