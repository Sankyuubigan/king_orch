@echo off
cd /d "%~dp0"

:: Kill old instance to avoid file lock during build
taskkill /f /im king_orch.exe 2>nul

:: Reset sccache-wrapped compilers (see desktop_rust_tauri/rules.md:61-63)
set "CC="
set "CXX="
set "CMAKE_C_COMPILER_LAUNCHER="
set "RUSTC_WRAPPER="
set "CARGO_BUILD_RUSTC_WRAPPER="

:: Auto-detect and init MSVC (Visual Studio Build Tools)
for /f "usebackq delims=" %%i in (`"%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe" -latest -products * -legacy -property installationPath 2^>nul`) do (
    if exist "%%i\VC\Auxiliary\Build\vcvarsall.bat" (
        call "%%i\VC\Auxiliary\Build\vcvarsall.bat" x64 >nul 2>&1
    )
)

echo ========================================
echo   King Orch - Installer Generation
echo ========================================

:: Step 1: Prep (npm install, sidecars, icons, version bump)
echo [1/4] Preparing environment...
echo.
node build.cjs --prep-only
if %ERRORLEVEL% NEQ 0 (
    echo.
    echo ========================================
    echo   PREP ERROR!
    echo ========================================
    pause >nul
    exit /b 1
)

:: Steps 2-4: installer.cjs (key + tauri build + sidecars + extract bundle)
node installer.cjs
if %ERRORLEVEL% NEQ 0 (
    echo.
    echo ========================================
    echo   BUILD ERROR!
    echo ========================================
    pause >nul
    exit /b 1
)

echo.
echo ========================================
echo   DONE! Press any key to exit...
echo ========================================
pause >nul
exit /b 0