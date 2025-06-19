@echo off
rem ===================================================================================
rem ПОЛНЫЙ СБРОС (CPU-ТЕСТ ВЕРСИЯ ДЛЯ ДИАГНОСТИКИ)
rem ===================================================================================
setlocal
chcp 65001 > nul

rem --- НАСТРОЙКИ ---
set "ProjectDir=%~dp0Orchestrator"
set "ArchiveDir=%~dp0Image_Archives"
set "LocalModelDir=D:/nn/models"

rem --- Настройки Docker образов ---
set "OllamaImage=docker.io/ollama/ollama:latest"
set "OllamaArchiveFile=ollama.tar"
set "AppImage=ghcr.io/open-webui/open-webui:main"
set "AppArchiveFile=open-webui.tar"

rem --- НАЧАЛО СКРИПТА ---

echo.
echo --- [ШАГ 1/4] Подготовка окружения (CPU-ТЕСТ ВЕРСИЯ)... ---

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
    rem --- GPU ВРЕМЕННО ОТКЛЮЧЕН ДЛЯ ДИАГНОСТИКИ ---
    rem echo     deploy:
    rem echo       resources:
    rem echo         reservations:
    rem echo           devices:
    rem echo             - driver: nvidia
    rem echo               count: all
    rem echo               capabilities: [gpu]
    echo     volumes:
    echo       - ollama_data:/root/.ollama
    echo       - "%LocalModelDir%:/models"
    echo     ports:
    echo       - "11434:11434"
    echo     networks:
    echo       - ai_net
    echo     restart: always
    echo.
    echo   open-webui:
    echo     image: %AppImage%
    echo     container_name: open-webui
    echo     depends_on:
    echo       - ollama
    echo     ports:
    echo       - "8080:8080"
    echo     environment:
    echo       - OLLAMA_BASE_URL=http://ollama:11434
    echo     volumes:
    echo       - open_webui_data:/app/backend/data
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
    echo   open_webui_data:
) > "%ProjectDir%\docker-compose.yml"

echo docker-compose.yml успешно создан для CPU-теста.
echo.

rem --- Остальная часть скрипта остается без изменений ---
echo --- [ШАГ 2/4] Проверка локальных архивов образов... ---
set "OllamaArchivePath=%ArchiveDir%\%OllamaArchiveFile%"
set "AppArchivePath=%ArchiveDir%\%AppArchiveFile%"
if not exist "%OllamaArchivePath%" set "NeedsPull=true"
if not exist "%AppArchivePath%" set "NeedsPull=true"
if defined NeedsPull (
    echo Один или несколько архивов не найдены. Запускается фаза 'Скачивание и Архивация'.
    docker login
    if %errorlevel% neq 0 ( echo ERROR: Docker login не удался. & goto :eof )
    if not exist "%OllamaArchivePath%" (
        echo Скачиваю образ Ollama... & docker pull %OllamaImage%
        if %errorlevel% neq 0 ( echo ERROR: Не удалось скачать образ Ollama. & goto :eof )
        echo Сохраняю Ollama в архив... & docker save -o "%OllamaArchivePath%" %OllamaImage%
        if %errorlevel% neq 0 ( echo ERROR: Не удалось сохранить образ Ollama. & goto :eof )
    )
    if not exist "%AppArchivePath%" (
        echo Скачиваю образ Open WebUI... & docker pull %AppImage%
        if %errorlevel% neq 0 ( echo ERROR: Не удалось скачать образ Open WebUI. & goto :eof )
        echo Сохраняю Open WebUI в архив... & docker save -o "%AppArchivePath%" %AppImage%
        if %errorlevel% neq 0 ( echo ERROR: Не удалось сохранить образ Open WebUI. & goto :eof )
    )
    docker logout & echo Архивы успешно созданы.
) else (
    echo "Все необходимые архивы уже существуют. Фаза 'Скачивание и Архивация' пропущена."
)
echo.

echo --- [ШАГ 3/4] Развертывание системы из локальных данных... ---
cd /d "%ProjectDir%"
echo Выполняется полная очистка предыдущей установки...
timeout /t 3 > nul
docker compose down --volumes --remove-orphans > nul 2>&1
echo Загрузка Ollama из архива...
docker load -i "%OllamaArchivePath%"
if %errorlevel% neq 0 ( echo ERROR: Не удалось загрузить Ollama из архива. & goto :eof )
echo Загрузка Open WebUI из архива...
docker load -i "%AppArchivePath%"
if %errorlevel% neq 0 ( echo ERROR: Не удалось загрузить Open WebUI из архива. & goto :eof )
echo.

echo --- [ШАГ 4/4] Запуск системы... ---
docker compose up -d
if %errorlevel% neq 0 ( echo ERROR: Docker Compose не смог запуститься. & goto :eof )
echo Система с Open WebUI успешно запущена в режиме CPU!
echo Подождите 1-2 минуты для полной инициализации сервисов.
echo Затем откройте браузер: http://localhost:8080
:eof
endlocal