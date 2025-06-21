# main.py - ДОБАВЛЕН ЗАПУСК НОВЫХ СЕРВЕРОВ GITHUB И GITLAB

import tkinter as tk
from tkinter import messagebox
import subprocess
import atexit
import sys
import os
import threading
import time

from ui import AppUI
from engine import OrchestratorEngine

tool_processes = []

def create_dirs():
    os.makedirs("agents", exist_ok=True)
    os.makedirs("crews", exist_ok=True)
    os.makedirs("mcp_servers", exist_ok=True)
    os.makedirs("workspace", exist_ok=True)
    for d in ["agents", "crews"]:
        with open(os.path.join(d, "__init__.py"), "a"): pass

def start_tool_server(command, log_prefix, ready_signal):
    global tool_processes
    script_path = command[-1]
    if not os.path.exists(script_path): return False, f"[{log_prefix}] ERROR: Файл '{script_path}' не найден."
    log_messages = []
    process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, encoding='utf-8', errors='ignore', creationflags=subprocess.CREATE_NO_WINDOW)
    tool_processes.append(process)
    log_messages.append(f"[{log_prefix}] Сервер запущен (PID: {process.pid})")
    initial_signal_ok = False
    for _ in range(20):
        if process.poll() is not None: break
        line = process.stdout.readline().strip()
        if line:
            print(f"[{log_prefix}] {line}")
            log_messages.append(f"[{log_prefix}] {line}")
            if ready_signal in line:
                initial_signal_ok = True
                break
    if not initial_signal_ok:
        log_messages.append(f"[{log_prefix}] ERROR: Не дождались сигнала готовности.")
        return False, "\n".join(log_messages)
    threading.Thread(target=lambda: [print(f"[{log_prefix}] {l.strip()}") for l in iter(process.stdout.readline, '') if l], daemon=True).start()
    return True, "\n".join(log_messages)

def stop_all_tool_servers():
    print("[Launcher] Остановка всех MCP-серверов...")
    for process in tool_processes:
        if process.poll() is None:
            try: process.terminate()
            except ProcessLookupError: pass
    time.sleep(0.5)
    for process in tool_processes:
        if process.poll() is None:
            try: process.kill()
            except ProcessLookupError: pass
    print("[Launcher] Все MCP-серверы остановлены.")

atexit.register(stop_all_tool_servers)

if __name__ == "__main__":
    create_dirs()
    main_window = tk.Tk()
    all_servers_ok, startup_log = True, []
    
    servers_to_start = {
        "WebSearch": ("mcp_servers/mcp_search_server.py", "MCP_SEARCH_READY"),
        "BrowserAgent": ("mcp_servers/mcp_browser_server.py", "MCP_SERVER_READY"),
        "Fetcher": ("mcp_servers/mcp_fetcher_server.py", "MCP_FETCHER_READY"),
        "FileSystem": ("mcp_servers/mcp_file_server.py", "MCP_FILE_SYSTEM_READY"),
        "CodeRunner": ("mcp_servers/mcp_code_runner.py", "MCP_CODE_RUNNER_READY"),
        "GitHub": ("mcp_servers/mcp_github_server.py", "MCP_GITHUB_READY"),
        "GitLab": ("mcp_servers/mcp_gitlab_server.py", "MCP_GITLAB_READY")
    }

    for name, (script, signal) in servers_to_start.items():
        success, logs = start_tool_server([sys.executable, "-u", script], name, signal)
        startup_log.append(logs)
        all_servers_ok &= success

    if not all_servers_ok:
        log_string = '\n\n'.join(startup_log)
        messagebox.showerror("Критическая ошибка", f"Не удалось запустить один или несколько MCP-серверов.\n\nЛог:\n{log_string}")
        sys.exit(1)

    engine = OrchestratorEngine(log_callback=lambda msg: print(f"[Engine Log] {msg}"))
    app = AppUI(main_window, engine)
    engine.log = app.log_to_widget
    for log_block in startup_log:
        for log_entry in log_block.splitlines():
            if log_entry: app.log_to_widget(log_entry)
    
    main_window.mainloop()