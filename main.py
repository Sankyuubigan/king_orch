# main.py - ВЕРСИЯ С HTTP HEALTH CHECK

import tkinter as tk
from tkinter import messagebox
import subprocess
import atexit
import sys
import os
import threading
import time
import requests

from ui import AppUI
from engine import OrchestratorEngine

mcp_process = None
HEALTH_CHECK_URL = "http://127.0.0.1:7777/health"

def start_mcp_server(log_callback):
    global mcp_process
    script_path = "mcp_server.js"
    
    if not os.path.exists(script_path):
        log_callback(f"[Launcher] ERROR: Файл сервера '{script_path}' не найден.")
        return False

    log_callback("[Launcher] Запуск MCP-сервера...")
    mcp_process = subprocess.Popen(
        ["node", script_path],
        stdout=subprocess.PIPE, 
        stderr=subprocess.STDOUT,
        text=True, 
        encoding='utf-8',
        creationflags=subprocess.CREATE_NO_WINDOW 
    )
    log_callback(f"[Launcher] MCP-сервер запущен с PID процесса Node: {mcp_process.pid}")

    initial_signal_ok = False
    for line in iter(mcp_process.stdout.readline, ''):
        stripped_line = line.strip()
        log_callback(stripped_line)
        if "MCP_SERVER_READY_FOR_CONNECTIONS" in stripped_line:
            initial_signal_ok = True
            log_callback("[Launcher] Сервер подал сигнал готовности. Начинаем HTTP Health Check...")
            break
        if "КРИТИЧЕСКАЯ ОШИБКА" in stripped_line:
             log_callback("[Launcher] ERROR: MCP-сервер не смог запуститься.")
             return False
    
    if not initial_signal_ok:
        log_callback("[Launcher] ERROR: Не дождались сигнала готовности от сервера.")
        return False

    max_retries = 20
    for i in range(max_retries):
        try:
            response = requests.get(HEALTH_CHECK_URL, timeout=1)
            if response.status_code == 200 and response.text == 'OK':
                log_callback(f"[Launcher] УСПЕХ! Health Check пройден на попытке №{i+1}.")
                threading.Thread(target=lambda: [log_callback(l.strip()) for l in iter(mcp_process.stdout.readline, '')], daemon=True).start()
                return True
        except requests.ConnectionError:
            log_callback(f"[Launcher] Health Check: попытка №{i+1} не удалась. Ждем...")
            time.sleep(0.5)
    
    log_callback("[Launcher] ERROR: Сервер не прошел Health Check после всех попыток.")
    return False


def stop_mcp_server():
    if mcp_process:
        print("[Launcher] Остановка MCP-сервера...")
        mcp_process.terminate()
        try: mcp_process.wait(timeout=5)
        except subprocess.TimeoutExpired: mcp_process.kill()
        print("[Launcher] MCP-сервер остановлен.")

atexit.register(stop_mcp_server)

if __name__ == "__main__":
    main_window = tk.Tk()
    
    temp_log = []
    server_ok = start_mcp_server(lambda msg: temp_log.append(msg) or print(msg))

    if not server_ok:
        error_message = "\n".join(temp_log)
        messagebox.showerror("Критическая ошибка", f"Не удалось запустить и проверить MCP-сервер.\n\nЛог:\n{error_message}")
        sys.exit(1)

    engine = OrchestratorEngine(log_callback=lambda msg: print(f"[Engine Log] {msg}"))
    app = AppUI(main_window, engine)
    
    engine.log = app.log_to_widget
    for log_entry in temp_log:
        app.log_to_widget(log_entry)
    
    main_window.mainloop()