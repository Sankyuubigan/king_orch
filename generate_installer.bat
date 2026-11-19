@echo off
setlocal enableextensions

set "PROJ=%~dp0"
if not exist "%PROJ%src-tauri\tauri.conf.json" (
  echo [ERROR] This script must live in the project root, next to src-tauri\tauri.conf.json
  pause
  exit /b 1
)

set "TOOLKIT=%BUILD_TOOLKIT%"
if "%TOOLKIT%"=="" set "TOOLKIT=%~dp0..\cross-app-plugins\rust-build-toolkit\cli.cjs"
if not exist "%TOOLKIT%" (
  echo [ERROR] Tauri build toolkit not found: "%TOOLKIT%"
  echo Set env BUILD_TOOLKIT to the toolkit cli.cjs, or place the
  echo toolkit folder at: cross-app-plugins\rust-build-toolkit
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

REM --- Шлюз cloud-routers: собрать и уложить в ресурсы бандла (19.11.2026) ---
REM Тот же прe-шаг, что в build.bat: инсталлер пакует бинарь рядом с exe хоста
REM (tauri.conf.json → resources), откуда installer плагина копирует его юзеру
REM (source_candidates, кандидат 1 — gateway_installer.rs). Без этого шага
REM установка дашборда на машинах юзеров невозможна в принципе.
set "GW_MANIFEST=%~dp0..\cross-app-plugins\Cargo.toml"
if not exist "%GW_MANIFEST%" (
  echo [ERROR] cross-app-plugins workspace not found: "%GW_MANIFEST%"
  pause
  exit /b 1
)
cargo build --manifest-path "%GW_MANIFEST%" -p cloud-routers-gateway --release
if errorlevel 1 (
  echo [ERROR] cloud-routers-gateway build failed.
  pause
  exit /b 1
)
set "GW_TARGET="
for /f "usebackq delims=" %%i in (`powershell -NoProfile -Command "(cargo metadata --manifest-path '%GW_MANIFEST%' --no-deps --format-version 1 | ConvertFrom-Json).target_directory"`) do set "GW_TARGET=%%i"
if "%GW_TARGET%"=="" (
  echo [ERROR] cargo metadata did not report target_directory.
  pause
  exit /b 1
)
if not exist "%GW_TARGET%\release\cloud-routers-gateway.exe" (
  echo [ERROR] Gateway binary not found after build: %GW_TARGET%\release\cloud-routers-gateway.exe
  pause
  exit /b 1
)
if not exist "%PROJ%src-tauri\resources" mkdir "%PROJ%src-tauri\resources"
copy /Y "%GW_TARGET%\release\cloud-routers-gateway.exe" "%PROJ%src-tauri\resources\cloud-routers-gateway.exe" >nul
echo [gateway] copied to src-tauri\resources from %GW_TARGET%\release

node "%TOOLKIT%" installer --project "%PROJ%"
if errorlevel 1 (
  echo [ERROR] Installer build failed.
  pause
  exit /b 1
)

echo [+DONE] generate_installer.bat finished.
endlocal