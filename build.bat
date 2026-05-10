@echo off
cd /d "%~dp0"

echo ========================================
echo Updating version to Year.Month.Build...
echo ========================================
powershell -Command "$confPath='src-tauri\tauri.conf.json'; $confText=[System.IO.File]::ReadAllText($confPath); $confText -match '\"version\"\s*:\s*\"(\d+)\.(\d+)\.(\d+)\"' | Out-Null; $oldMaj=$matches[1]; $oldMin=$matches[2]; $oldPat=[int]$matches[3]; $newMaj=(Get-Date).ToString('yy'); $newMin=[int]((Get-Date).ToString('MM')); if ($oldMaj -eq $newMaj -and $oldMin -eq $newMin) { $newPat = $oldPat + 1 } else { $newPat = 1 }; $version=\"$newMaj.$newMin.$newPat\"; $utf8NoBom = New-Object System.Text.UTF8Encoding $false; $confText=$confText -replace '\"version\"\s*:\s*\".*?\"', \"`\"version`\": `\"$version`\"\";[System.IO.File]::WriteAllText($confPath, $confText, $utf8NoBom); $cargoPath='src-tauri\Cargo.toml'; $cargoText=[System.IO.File]::ReadAllText($cargoPath); $cargoText=$cargoText -replace '(?m)^version\s*=\s*\".*?\"', \"version = `\"$version`\"\";[System.IO.File]::WriteAllText($cargoPath, $cargoText, $utf8NoBom); Write-Host \"Local version updated to: $version\""

echo.
echo ========================================
echo Installing Node.js dependencies...
echo ========================================
call npm install

echo.
echo ========================================
echo Building Tauri application...
echo ========================================

set "TAURI_PRIVATE_KEY=%~dp0~\.tauri\bookmarks.key"
set "TAURI_KEY_PASSWORD=123"

call npx tauri build
if errorlevel 1 goto build_error

echo.
echo ========================================
echo Build successful! Starting application...
echo ========================================
start "" "src-tauri\target\release\king_orch.exe"
goto :eof

:build_error
echo.
echo ========================================
echo ERROR: Build failed!
echo Please check the error messages above.
echo ========================================
pause
exit /b 1