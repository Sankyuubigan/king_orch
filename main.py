# main.py - ОБНОВЛЕННАЯ ВЕРСИЯ С ПЕРЕХВАТОМ ЛОГОВ

import tkinter as tk
import subprocess
import atexit
import sys
import os
import threading

from ui import AppUI
from engine import OrchestratorEngine

# --- Логика автозапуска и остановки MCP-сервера ---
mcp_process = None
log_thread = None

def _log_mcp_output(pipe, log_callback):
    """Читает вывод из потока сервера и передает его в логгер UI."""
    try:
        # Читаем построчно, пока процесс не завершится
        for line in iter(pipe.readline, ''):
            log_callback(f"[MCP Server] {line.strip()}")
    finally:
        pipe.close()

def start_mcp_server(log_callback):
    """Запускает MCP-сервер и потоки для логирования его вывода."""
    global mcp_process, log_thread
    script_path = "mcp_server.js"
    
    if not os.path.exists(script_path):
        log_callback(f"[ERROR] Файл сервера '{script_path}' не найден.")
        return

    try:
        log_callback("[Launcher] Запуск MCP-сервера...")
        # <<< ИЗМЕНЕНИЕ: Перехватываем stdout и stderr >>>
        mcp_process = subprocess.Popen(
            ["node", script_path],
            stdout=subprocess.PIPE, 
            stderr=subprocess.PIPE, # Перехватываем и ошибки
            text=True, 
            encoding='utf-8',
            # Важно для Windows, чтобы консольное окно не появлялось
            creationflags=subprocess.CREATE_NO_WINDOW 
        )
        log_callback(f"[Launcher] MCP-сервер запущен с PID: {mcp_process.pid}")

        # <<< НОВОЕ: Запускаем поток для чтения логов сервера >>>
        log_thread = threading.Thread(
            target=_log_mcp_output, 
            args=(mcp_process.stdout, log_callback), 
            daemon=True
        )
        log_thread.start()
        
        # <<< НОВОЕ: Запускаем второй поток для чтения ошибок сервера >>>
        error_log_thread = threading.Thread(
            target=_log_mcp_output,
            args=(mcp_process.stderr, log_callback),
            daemon=True
        )
        error_log_thread.start()

    except Exception as e:
        log_callback(f"[ERROR] Не удалось запустить MCP-сервер: {e}")
        mcp_process = None

def stop_mcp_server():
    """Останавливает MCP-сервер при выходе из приложения."""
    global mcp_process
    if mcp_process:
        print("[Launcher] Остановка MCP-сервера...") # Вывод в консоль при закрытии
        mcp_process.terminate()
        mcp_process.wait()
        print("[Launcher] MCP-сервер остановлен.")

# Регистрируем функцию остановки
atexit.register(stop_mcp_server)

# --- Основная точка входа в приложение ---
if __name__ == "__main__":
    main_window = tk.Tk()
    
    # Создаем движок и UI
    engine = OrchestratorEngine(log_callback=lambda msg: print(f"[Engine Log] {msg}"))
    app = AppUI(main_window, engine)
    
    # <<< ИЗМЕНЕНИЕ: Передаем метод логирования из UI в движок и сервер >>>
    engine.log = app.log_to_widget
    
    # 1. Запускаем фоновый MCP-сервер, передавая ему колбэк для логов
    start_mcp_server(app.log_to_widget)
    
    # 2. Запускаем главный цикл приложения
    main_window.mainloop()