@echo off
setlocal enableextensions

set "PROJ=%~dp0"
if not exist "%PROJ%src-tauri\tauri.conf.json" (
  echo [ERROR] This script must live in the project root, next to src-tauri\tauri.conf.json
  pause
  exit /b 1
)

set "TOOLKIT=%TAURI_BUILD_TOOLKIT%"
if "%TOOLKIT%"=="" set "TOOLKIT=%~dp0..\my-tauri-plugins\tauri-build-toolkit\cli.cjs"
if not exist "%TOOLKIT%" (
  echo [ERROR] Tauri build toolkit not found: "%TOOLKIT%"
  echo Set env TAURI_BUILD_TOOLKIT to the toolkit cli.cjs, or place the
  echo toolkit folder at: my-tauri-plugins\tauri-build-toolkit
  pause
  exit /b 1
)

set "VS_INIT_OK="
for /f "usebackq delims=" %%i in (`"%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe" -latest -products * -property installationPath 2^>nul`) do (
    if exist "%%i\VC\Auxiliary\Build\vcvarsall.bat" (
        call "%%i\VC\Auxiliary\Build\vcvarsall.bat" x64 >nul 2>&1
        set "VS_INIT_OK=1"
    )
)
if not defined VS_INIT_OK (
  echo [ERROR] Visual Studio not found. vswhere. Install VS with C++ workload.
  pause
  exit /b 1
)

set "CC="
set "CXX="
set "CMAKE_C_COMPILER_LAUNCHER="
set "CMAKE_CXX_COMPILER_LAUNCHER="
set "RUSTC_WRAPPER="
set "CARGO_BUILD_RUSTC_WRAPPER="

set "CARGO_PROFILE_RELEASE_LTO="
set "CARGO_PROFILE_RELEASE_CODEGEN_UNITS="
set "CARGO_PROFILE_RELEASE_STRIP="

REM --- Temp of frontend build inside the project ---
REM The system %TEMP% on this machine denies DELETE of just-written files:
REM cl.exe fails with D8050, esbuild (inside vite) fails with
REM "remove ...esbuild-<hash>: Access is denied" -> vite build cannot finish.
REM A project-local temp dir is proven to build successfully. Keep it ASCII.
set "TMP_MISSING="
if not exist "%PROJ%temp" mkdir "%PROJ%temp"
set "FRONTEND_TMP=%PROJ%temp\buildtmp"
if not exist "%FRONTEND_TMP%" mkdir "%FRONTEND_TMP%"
if not exist "%FRONTEND_TMP%" set "TMP_MISSING=1"
if not defined TMP_MISSING (
  set "TMP=%FRONTEND_TMP%"
  set "TEMP=%FRONTEND_TMP%"
  echo [generate_installer.bat] temp for build: %FRONTEND_TMP%
) else (
  echo [generate_installer.bat] WARNING: cannot create %FRONTEND_TMP%, using system TEMP
)

node "%TOOLKIT%" installer --project "%PROJ%"
if errorlevel 1 (
  echo [ERROR] Installer build failed.
  pause
  exit /b 1
)

echo [+DONE] generate_installer.bat finished.
endlocal