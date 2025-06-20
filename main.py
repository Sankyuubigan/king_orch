# main.py - ВЕРСИЯ С ИСПРАВЛЕНИЕМ ОШИБКИ ДЕКОДИРОВАНИЯ (UnicodeDecodeError)

import tkinter as tk
from tkinter import messagebox
import subprocess
import atexit
import sys
import os
import threading
import time
import requests
import json

from ui import AppUI
from engine import OrchestratorEngine

tool_processes = []

def start_tool_server(command, log_prefix, ready_signal):
    global tool_processes
    
    script_path = command[-1]
    if not os.path.exists(script_path):
        return False, f"[{log_prefix}] ERROR: Файл сервера '{script_path}' не найден."

    log_messages = []
    
    # ИСПРАВЛЕНИЕ: Добавляем errors='ignore', чтобы избежать падения на не-UTF8 символах от дочерних процессов
    process = subprocess.Popen(
        command,
        stdout=subprocess.PIPE, 
        stderr=subprocess.STDOUT,
        text=True, 
        encoding='utf-8',
        errors='ignore', # <--- КЛЮЧЕВОЕ ИЗМЕНЕНИЕ
        creationflags=subprocess.CREATE_NO_WINDOW 
    )
    tool_processes.append(process)
    log_messages.append(f"[{log_prefix}] Сервер запущен с PID: {process.pid}")

    initial_signal_ok = False
    while True:
        line = process.stdout.readline()
        if not line and process.poll() is not None:
            break
            
        stripped_line = line.strip()
        if not stripped_line:
            continue

        print(f"[{log_prefix}] {stripped_line}")
        log_messages.append(f"[{log_prefix}] {stripped_line}")
        
        if ready_signal in stripped_line:
            initial_signal_ok = True
            break
        if "ERROR" in stripped_line.upper() or "КРИТИЧЕСКАЯ ОШИБКА" in stripped_line:
             log_messages.append(f"[{log_prefix}] ERROR: Сервер не смог запуститься.")
             return False, "\n".join(log_messages)
    
    if not initial_signal_ok:
        log_messages.append(f"[{log_prefix}] ERROR: Не дождались сигнала готовности от сервера.")
        remaining_output = process.stdout.read()
        if remaining_output:
            log_messages.append("[ПОЛНЫЙ ВЫВОД ПРОЦЕССА]:\n" + remaining_output)
        return False, "\n".join(log_messages)
        
    threading.Thread(target=lambda: [print(f"[{log_prefix}] {l.strip()}") for l in iter(process.stdout.readline, '') if l], daemon=True).start()
    
    return True, "\n".join(log_messages)


def stop_all_tool_servers():
    global tool_processes
    if tool_processes:
        print("[Launcher] Остановка всех MCP-серверов...")
        for process in tool_processes:
            if process.poll() is None:
                try:
                    process.terminate()
                except ProcessLookupError: pass
        time.sleep(0.5)
        for process in tool_processes:
            if process.poll() is None:
                try:
                    process.kill()
                except ProcessLookupError: pass
        print("[Launcher] Все MCP-серверы остановлены.")

atexit.register(stop_all_tool_servers)

if __name__ == "__main__":
    main_window = tk.Tk()
    
    if not os.path.exists("requirements.txt"):
        with open("requirements.txt", "w") as f:
            f.write("requests\nllama-cpp-python\nPillow\nplaywright\nfastapi\nuvicorn\n")
        messagebox.showinfo("Информация", "Создан файл requirements.txt. Пожалуйста, установите зависимости:\npip install -r requirements.txt\n\nА также браузеры для Playwright:\nplaywright install")
        sys.exit(0)

    all_servers_ok = True
    startup_log = []
    
    browser_server_command = [sys.executable, "-u", "mcp_browser_server.py"]
    success, logs = start_tool_server(browser_server_command, "BrowserMCP", "MCP_SERVER_READY")
    startup_log.append(logs)
    if not success:
        all_servers_ok = False

    if not all_servers_ok:
        error_message = "\n\n".join(startup_log)
        messagebox.showerror("Критическая ошибка", f"Не удалось запустить один или несколько MCP-серверов.\n\nЛог:\n{error_message}")
        sys.exit(1)

    engine = OrchestratorEngine(log_callback=lambda msg: print(f"[Engine Log] {msg}"))
    app = AppUI(main_window, engine)
    
    engine.log = app.log_to_widget
    for log_entry in "\n".join(startup_log).splitlines():
        if log_entry:
            app.log_to_widget(log_entry)
    
    main_window.mainloop()