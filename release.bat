@echo off
cd /d "%~dp0"

:: Kill old instance to avoid file lock during build
taskkill /f /im king_orch.exe 2>nul

:: Reset sccache-wrapped compilers
set "CC="
set "CXX="
set "CMAKE_C_COMPILER_LAUNCHER="
set "RUSTC_WRAPPER="
set "CARGO_BUILD_RUSTC_WRAPPER="

:: Profile optimizations are defined in src-tauri/Cargo.toml → [profile.release]
:: (single source of truth; unset env to keep cargo cache shared between scripts)
set "CARGO_PROFILE_RELEASE_LTO="
set "CARGO_PROFILE_RELEASE_CODEGEN_UNITS="
set "CARGO_PROFILE_RELEASE_STRIP="

:: Auto-detect and init MSVC (Visual Studio Build Tools)
for /f "usebackq delims=" %%i in (`"%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe" -latest -products * -legacy -property installationPath 2^>nul`) do (
    if exist "%%i\VC\Auxiliary\Build\vcvarsall.bat" (
        call "%%i\VC\Auxiliary\Build\vcvarsall.bat" x64 >nul 2>&1
    )
)

echo ========================================
echo   King Orch - Release Build ^& Publish
echo ========================================
node release.cjs
if %ERRORLEVEL% NEQ 0 pause