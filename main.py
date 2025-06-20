# main.py - ИСПРАВЛЕНАЯ ВЕРСИЯ С ЧИСТЫМ ЛОГИРОВАНИЕМ

import tkinter as tk
import subprocess
import atexit
import sys
import os
import threading

from ui import AppUI
from engine import OrchestratorEngine

mcp_process = None
log_thread = None

def _log_mcp_output(pipe, log_callback):
    """Читает вывод из потока сервера и передает его в логгер UI."""
    try:
        for line in iter(pipe.readline, ''):
            # <<< ИЗМЕНЕНИЕ: Убран лишний префикс [MCP Server] >>>
            # Теперь в лог передается "чистая" строка от самого сервера.
            log_callback(line.strip())
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
        mcp_process = subprocess.Popen(
            ["node", script_path],
            stdout=subprocess.PIPE, 
            stderr=subprocess.PIPE,
            text=True, 
            encoding='utf-8',
            creationflags=subprocess.CREATE_NO_WINDOW 
        )
        log_callback(f"[Launcher] MCP-сервер запущен с PID: {mcp_process.pid}")

        log_thread = threading.Thread(
            target=_log_mcp_output, 
            args=(mcp_process.stdout, log_callback), 
            daemon=True
        )
        log_thread.start()
        
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
        print("[Launcher] Остановка MCP-сервера...")
        mcp_process.terminate()
        try:
            mcp_process.wait(timeout=5)
            print("[Launcher] MCP-сервер штатно остановлен.")
        except subprocess.TimeoutExpired:
            print("[Launcher] MCP-сервер не ответил на terminate, принудительное завершение...")
            mcp_process.kill()
            print("[Launcher] MCP-сервер принудительно завершен.")


atexit.register(stop_mcp_server)

if __name__ == "__main__":
    main_window = tk.Tk()
    
    engine = OrchestratorEngine(log_callback=lambda msg: print(f"[Engine Log] {msg}"))
    app = AppUI(main_window, engine)
    
    engine.log = app.log_to_widget
    
    start_mcp_server(app.log_to_widget)
    
    main_window.mainloop()