@echo off
rem ===================================================================================
rem ДОБАВЛЕНИЕ ЛОКАЛЬНОЙ МОДЕЛИ В ПОРТАТИВНУЮ OLLAMA
rem ===================================================================================
setlocal
chcp 65001 > nul

rem --- НАСТРОЙКИ ---

rem [1] Полный путь к твоему ollama.exe
set "OllamaExePath=D:\Projects\universal_orchestrator\ollama\ollama.exe"

rem [2] Папка, где лежит твой GGUF файл
set "ModelFolder=D:\nn\models"

rem [3] ТОЧНОЕ имя твоего GGUF файла
set "GGUF_File=cognitivecomputations_Dolphin-Mistral-24B-Venice-Edition-Q4_K_M.gguf"

rem [4] Короткое имя для этой модели, которое ты будешь видеть в чате
set "NewModelName=dolphin-mistral-24b"


rem --- НЕ ТРОГАЙ НИЧЕГО НИЖЕ ---
set "FullPathToGGUF=%ModelFolder%\%GGUF_File%"
set "TempModelfileName=temp_modelfile.txt"
set "FullPathToModelfile=%ModelFolder%\%TempModelfileName%"

echo.
echo --- [1/4] Проверка файлов... ---

if not exist "%OllamaExePath%" (
    echo ERROR: Не найден ollama.exe! Проверь путь:
    echo %OllamaExePath%
    goto :eof
)
if not exist "%FullPathToGGUF%" (
    echo ERROR: Не найден файл модели GGUF! Проверь путь:
    echo %FullPathToGGUF%
    goto :eof
)
echo OK. Все файлы на месте.

echo.
echo --- [2/4] Создание временного Modelfile... ---
(echo FROM "%FullPathToGGUF%") > "%FullPathToModelfile%"
echo OK. Временный файл создан.

echo.
echo --- [3/4] Импорт модели в Ollama... ---
echo Название: %NewModelName%
echo.
echo !!! ЭТО МОЖЕТ ЗАНЯТЬ НЕСКОЛЬКО МИНУТ. OLLAMA ОБРАБАТЫВАЕТ МОДЕЛЬ... !!!
echo.

call "%OllamaExePath%" create %NewModelName% -f "%FullPathToModelfile%"

if %errorlevel% neq 0 (
    echo.
    echo ERROR: Ollama не смогла создать модель.
    echo Возможные причины:
    echo  - Недостаточно VRAM/RAM для обработки модели.
    echo  - Поврежденный GGUF файл.
    del "%FullPathToModelfile%" >nul 2>&1
    goto :eof
)

echo.
echo --- [4/4] Очистка... ---
del "%FullPathToModelfile%"
echo OK. Временный файл удален.

echo.
echo ===================================================================================
echo --- ПОБЕДА, БЛЯДЬ! ---
echo Модель "%NewModelName%" успешно добавлена в твою Ollama.
echo ===================================================================================
echo.

:eof
endlocal