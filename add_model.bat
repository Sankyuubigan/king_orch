chcp 65001 > nul
@echo off
setlocal

rem ===================================================================================
rem СКРИПТ ДЛЯ АВТОМАТИЧЕСКОГО ДОБАВЛЕНИЯ ЛОКАЛЬНОЙ GGUF МОДЕЛИ В OLLAMA
rem ЗАПУСКАТЬ ПОСЛЕ ТОГО, КАК СИСТЕМА УЖЕ ЗАПУЩЕНА ЧЕРЕЗ DEPLOY.BAT
rem ===================================================================================

rem --- НАСТРОЙКИ ---
rem [1] Укажите путь к вашей папке с моделями.
set "ModelFolder=D:/nn/models"

rem [2] Укажите ТОЧНОЕ имя вашего GGUF файла.
set "GGUF_File=Qwen3-64k-30B-A1.5B-NEO-MAX-IQ4_NL.gguf"

rem [3] Придумайте КОРОТКОЕ имя для модели, которое будет отображаться в AnythingLLM.
set "NewModelName=qwen3-30b-iq4nl"

rem --- Не трогайте ничего ниже ---
set "TempModelfile=temp_modelfile_for_ollama.txt"
set "FullPathToGGUF=%ModelFolder%\%GGUF_File%"
set "FullPathToModelfile=%ModelFolder%\%TempModelfile%"

echo.
echo --- [1/4] Проверка окружения... ---
if not exist "%FullPathToGGUF%" (
    echo ERROR: Файл модели не найден по указанному пути!
    echo Проверьте путь и имя файла: %FullPathToGGUF%
    goto :eof
)
echo OK. Файл модели найден.

echo.
echo --- [2/4] Автоматическое создание временного Modelfile... ---
(echo FROM ./%GGUF_File%) > "%FullPathToModelfile%"
if %errorlevel% neq 0 (
    echo ERROR: Не удалось создать временный Modelfile. Проверьте права доступа к папке.
    goto :eof
)
echo OK. Временный файл для Ollama создан.

echo.
echo --- [3/4] Добавление модели в Ollama... ---
echo Название модели: %NewModelName%
echo Исходный файл: %GGUF_File%
echo.
echo !!! ПРОЦЕСС МОЖЕТ ЗАНЯТЬ НЕСКОЛЬКО МИНУТ. ПОЖАЛУЙСТА, ПОДОЖДИТЕ... !!!
echo.

docker exec -it ollama ollama create %NewModelName% -f /models/%TempModelfile%

if %errorlevel% neq 0 (
    echo.
    echo ERROR: Не удалось создать модель в Ollama.
    echo Возможные причины:
    echo  - Недостаточно VRAM/RAM для обработки модели.
    echo  - Поврежденный GGUF файл.
    echo  - Контейнер ollama не запущен.
    del "%FullPathToModelfile%" >nul 2>&1
    goto :eof
)

echo.
echo --- [4/4] Очистка... ---
del "%FullPathToModelfile%"
echo OK. Временный Modelfile удален.

echo.
echo ===================================================================================
echo --- ГОТОВО! ---
echo Модель "%NewModelName%" успешно добавлена в Ollama.
echo Теперь вы можете зайти в AnythingLLM, открыть настройки рабочего пространства
echo и выбрать эту модель в выпадающем списке.
echo ===================================================================================
echo.

:eof
endlocal