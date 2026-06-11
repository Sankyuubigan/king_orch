@echo off
cd /d "%~dp0"

:: Auto-detect and init MSVC (Visual Studio Build Tools)
for /f "usebackq delims=" %%i in (`"%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe" -latest -products * -legacy -property installationPath 2^>nul`) do (
    if exist "%%i\VC\Auxiliary\Build\vcvarsall.bat" (
        call "%%i\VC\Auxiliary\Build\vcvarsall.bat" x64 >nul 2>&1
    )
)

echo ========================================
echo   King Orch - Installer Generation
echo ========================================

:: Step 1: Prep (npm install, sidecars, icons)
echo [1/4] Preparing environment...
echo.
node build.cjs
if %ERRORLEVEL% NEQ 0 (
    echo.
    echo ========================================
    echo   PREP ERROR! Press any key...
    echo ========================================
    pause >nul
    exit /b 1
)

:: Step 2: Signing key setup
echo ========================================
echo [2/4] Setting up signing key...
echo ========================================
echo.

set "DEFAULT_KEY_PATH=D:\Projects\docusaurus-starter\docs\Sega Mega Note\Моя картотека\software\настройки\tauri_signed_keys\tauri.key"
if defined TAURI_PRIVATE_KEY_ORIGINAL (
    set "KEY_PATH=%TAURI_PRIVATE_KEY_ORIGINAL%"
) else (
    set "KEY_PATH=%DEFAULT_KEY_PATH%"
)

if not exist "%KEY_PATH%" (
    echo WARNING: Signing key not found: "%KEY_PATH%"
    echo    Building without signature (local testing only).
    set "TAURI_SIGNING_PRIVATE_KEY="
    set "TAURI_SIGNING_PRIVATE_KEY_PASSWORD="
) else (
    echo Signing key found.
    for /f "usebackq delims=" %%k in (`powershell -NoProfile -Command "$c=(Get-Content '%KEY_PATH%' -Raw).Trim() -replace '[\r\n]+',''; Write-Output $c"`) do set "TAURI_SIGNING_PRIVATE_KEY=%%k"
    set "TAURI_SIGNING_PRIVATE_KEY_PASSWORD=123"
    echo Env vars set.
)

:: Step 3: Tauri build (generates king_orch.exe + NSIS installer)
echo.
echo ========================================
echo [3/4] Building Tauri app (release)...
echo ========================================
echo.

npx tauri build
if %ERRORLEVEL% NEQ 0 (
    echo.
    echo ========================================
    echo   BUILD ERROR!
    echo   Fix compilation errors and run generate_installer.bat again.
    echo ========================================
    pause >nul
    exit /b 1
)

:: Step 4: Post-build
echo.
echo ========================================
echo [4/4] Post-build operations...
echo ========================================

:: Copy sidecars next to exe
set "BIN_DIR=src-tauri\bin"
set "RELEASE_DIR=src-tauri\target\release"
set "RELEASE_BIN_DIR=%RELEASE_DIR%\bin"

if exist "%BIN_DIR%" if exist "%RELEASE_DIR%" (
    if not exist "%RELEASE_BIN_DIR%" mkdir "%RELEASE_BIN_DIR%"
    for %%f in ("%BIN_DIR%\*") do (
        copy /Y "%%f" "%RELEASE_BIN_DIR%\" >nul 2>&1
    )
    echo   Sidecars copied.
)

:: Verify exe exists
set "EXE_PATH=%RELEASE_DIR%\king_orch.exe"
if not exist "%EXE_PATH%" (
    echo.
    echo king_orch.exe not found! Build failed.
    pause >nul
    exit /b 1
)

:: Launch app
echo.
echo ========================================
echo Launching King Orch (no console)...
echo ========================================
start "" "%EXE_PATH%"
echo App launched! This window will close automatically.
timeout /t 2 >nul 2>&1
exit /b 0
