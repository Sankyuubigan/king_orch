# main.py - АКТУАЛЬНАЯ ВЕРСИЯ

import tkinter as tk
import subprocess
import atexit
import sys
import os

from ui import AppUI
from engine import OrchestratorEngine

# --- Логика автозапуска и остановки MCP-сервера ---
mcp_process = None

def start_mcp_server():
    """Запускает MCP-сервер как фоновый процесс."""
    global mcp_process
    script_path = "mcp_server.js"
    
    if not os.path.exists(script_path):
        print(f"ОШИБКА: Файл сервера '{script_path}' не найден.", file=sys.stderr)
        return

    try:
        print("[Launcher] Запуск MCP-сервера...")
        mcp_process = subprocess.Popen(
            ["node", script_path],
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, encoding='utf-8'
        )
        print(f"[Launcher] MCP-сервер запущен с PID: {mcp_process.pid}")
    except Exception as e:
        print(f"ОШИБКА: Не удалось запустить MCP-сервер: {e}", file=sys.stderr)
        mcp_process = None

def stop_mcp_server():
    """Останавливает MCP-сервер при выходе из приложения."""
    global mcp_process
    if mcp_process:
        print(f"[Launcher] Остановка MCP-сервера (PID: {mcp_process.pid})...")
        mcp_process.terminate()
        mcp_process.wait()
        print("[Launcher] MCP-сервер остановлен.")

# Регистрируем функцию остановки, чтобы она вызывалась при закрытии
atexit.register(stop_mcp_server)

# --- Основная точка входа в приложение ---
if __name__ == "__main__":
    # 1. Запускаем фоновый MCP-сервер
    start_mcp_server()
    
    # 2. Создаем и запускаем UI
    main_window = tk.Tk()
    
    # Создаем движок и связываем его с UI для логирования
    engine = OrchestratorEngine(log_callback=lambda msg: print(f"[Engine Log] {msg}"))
    app = AppUI(main_window, engine)
    engine.log = app.log_to_widget
    
    # 3. Запускаем главный цикл приложения
    main_window.mainloop()