@echo off
chcp 65001 > nul

echo.
echo =================================================================
echo           ЗАПУСК СИСТЕМЫ "ОРКЕСТРАТОР"
echo =================================================================
echo.
echo --- [1/2] Запускаю сервер Ollama в новом окне...
echo.

rem Запускаем Ollama в ОТДЕЛЬНОМ, ВИДИМОМ окне
start "Ollama Server" "D:\Projects\universal_orchestrator\ollama\ollama.exe" serve

echo --- [2/2] Запускаю Open WebUI в новом окне...
echo.

rem Запускаем Open WebUI в ОТДЕЛЬНОМ, ВИДИМОМ окне
rem Он сам активирует окружение и запустит сервер
start "Open WebUI Server" D:\Programs\miniconda3\Scripts\conda.exe run -p D:\Projects\universal_orchestrator\.venv open-webui serve

echo --- Ожидание запуска серверов (15 секунд)...
echo.
timeout /t 15 > nul

echo --- СЕРВЕРЫ ЗАПУЩЕНЫ В ОТДЕЛЬНЫХ ОКНАХ. ---
echo --- ОНИ ДОЛЖНЫ ОСТАВАТЬСЯ ОТКРЫТЫМИ. ---
echo.
echo --- ТЕПЕРЬ ОТКРОЙ БРАУЗЕР И ПЕРЕЙДИ ПО АДРЕСУ: ---
echo.
echo             http://localhost:8080
echo.
echo =================================================================

rem Этот скрипт теперь просто ждет, пока ты не закроешь его сам.
pause