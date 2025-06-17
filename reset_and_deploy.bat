chcp 65001 > nul
@echo off
rem ===================================================================================
rem ОСНОВНОЙ СКРИПТ РАЗВЕРТЫВАНИЯ СИСТЕМЫ
rem ЗАПУСКАТЬ ДЛЯ СТАРТА И ПЕРЕЗАПУСКА ВСЕЙ СИСТЕМЫ
rem ===================================================================================
setlocal

rem --- НАСТРОЙКИ ---
set "ProjectDir=%~dp0Orchestrator"
set "ArchiveDir=%~dp0Image_Archives"
set "LocalModelDir=D:/nn/models"

rem --- Настройки Docker образов ---
set "OllamaImage=docker.io/ollama/ollama:latest"
set "OllamaArchiveFile=ollama.tar"
set "AppImage=docker.io/mintplexlabs/anythingllm:latest"
set "AppArchiveFile=anything-llm.tar"

rem --- НАЧАЛО СКРИПТА ---

echo.
echo --- [ШАГ 1/4] Подготовка окружения и конфигурации... ---

if not exist "%ProjectDir%" mkdir "%ProjectDir%"
if not exist "%ArchiveDir%" mkdir "%ArchiveDir%"
if not exist "%LocalModelDir%" (
    echo ERROR: Папка для локальных моделей не найдена!
    echo Пожалуйста, создайте папку: %LocalModelDir%
    goto :eof
)

(
    echo services:
    echo   ollama:
    echo     image: %OllamaImage%
    echo     container_name: ollama
    echo     deploy:
    echo       resources:
    echo         reservations:
    echo           devices:
    echo             - driver: nvidia
    echo               count: all
    echo               capabilities: [gpu]
    echo     volumes:
    echo       - ollama_data:/root/.ollama
    echo       - "%LocalModelDir%:/models"
    echo     ports:
    echo       - "11434:11434"
    echo     networks:
    echo       - ai_net
    echo     restart: always
    echo.
    echo   anything-llm:
    echo     image: %AppImage%
    echo     container_name: anything-llm
    echo     depends_on:
    echo       - ollama
    echo     ports:
    echo       - "3001:3001"
    echo     environment:
    echo       - STORAGE_DIR=/app/server/storage
    echo     volumes:
    echo       - anything_storage:/app/server/storage
    echo     networks:
    echo       - ai_net
    echo     restart: always
    echo.
    echo networks:
    echo   ai_net:
    echo     name: ai_net_docker
    echo     driver: bridge
    echo.
    echo volumes:
    echo   ollama_data:
    echo   anything_storage:
) > "%ProjectDir%\docker-compose.yml"

echo docker-compose.yml успешно создан.
echo.

echo --- [ШАГ 2/4] Проверка локальных архивов образов... ---

set "OllamaArchivePath=%ArchiveDir%\%OllamaArchiveFile%"
set "AppArchivePath=%ArchiveDir%\%AppArchiveFile%"

if not exist "%OllamaArchivePath%" set "NeedsPull=true"
if not exist "%AppArchivePath%" set "NeedsPull=true"

if defined NeedsPull (
    echo Один или несколько архивов не найдены. Запускается фаза 'Скачивание и Архивация'.
    docker login
    if %errorlevel% neq 0 ( echo ERROR: Docker login не удался. & goto :eof )
    echo Скачиваю образ Ollama...
    docker pull %OllamaImage%
    if %errorlevel% neq 0 ( echo ERROR: Не удалось скачать образ Ollama. & goto :eof )
    echo Скачиваю образ AnythingLLM...
    docker pull %AppImage%
    if %errorlevel% neq 0 ( echo ERROR: Не удалось скачать образ AnythingLLM. & goto :eof )
    echo Сохраняю Ollama в архив...
    docker save -o "%OllamaArchivePath%" %OllamaImage%
    if %errorlevel% neq 0 ( echo ERROR: Не удалось сохранить образ Ollama. & goto :eof )
    echo Сохраняю AnythingLLM в архив...
    docker save -o "%AppArchivePath%" %AppImage%
    if %errorlevel% neq 0 ( echo ERROR: Не удалось сохранить образ AnythingLLM. & goto :eof )
    docker logout
    echo Архивы успешно созданы.
) else (
    echo "Все необходимые архивы уже существуют. Фаза 'Скачивание и Архивация' пропущена."
)
echo.

echo --- [ШАГ 3/4] Развертывание системы из локальных данных... ---

cd /d "%ProjectDir%"

echo Выполняется полная очистка предыдущей установки...
echo ВНИМАНИЕ: Это удалит все ранее скачанные модели и данные!
echo Нажмите CTRL+C в течение 5 секунд для отмены.
timeout /t 5 > nul
docker compose down --volumes --remove-orphans > nul 2>&1

echo Загрузка Ollama из архива...
docker load -i "%OllamaArchivePath%"
if %errorlevel% neq 0 ( echo ERROR: Не удалось загрузить Ollama из архива. & goto :eof )

echo Загрузка AnythingLLM из архива...
docker load -i "%AppArchivePath%"
if %errorlevel% neq 0 ( echo ERROR: Не удалось загрузить AnythingLLM из архива. & goto :eof )
echo.

echo --- [ШАГ 4/4] Запуск системы... ---

docker compose up -d
if %errorlevel% neq 0 ( echo ERROR: Docker Compose не смог запуститься. & goto :eof )

echo Система успешно запущена!
echo Подождите 1-2 минуты для полной инициализации сервисов.
echo Затем откройте браузер: http://localhost:3001

:eof
endlocal