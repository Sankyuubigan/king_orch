# main.py - Голосовой движок запускается, но не слушает сразу

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
from voice_engine.controller import VoiceController

tool_processes = []

def create_dirs():
    os.makedirs("agents", exist_ok=True)
    os.makedirs("crews", exist_ok=True)
    os.makedirs("mcp_servers", exist_ok=True)
    os.makedirs("prompts", exist_ok=True)
    os.makedirs("utils", exist_ok=True)
    os.makedirs("tools", exist_ok=True)
    # Создаем папки по вашей структуре
    os.makedirs("voice_engine/vosk", exist_ok=True)
    os.makedirs("voice_engine/silero/hub_cache", exist_ok=True)
    for d in ["agents", "crews", "utils", "voice_engine"]:
        # Создаем __init__.py, если его нет
        init_file = os.path.join(d, "__init__.py")
        if not os.path.exists(init_file):
            with open(init_file, "a"): pass

def start_tool_server(command, log_prefix, ready_signal):
    # ... (код без изменений)
    global tool_processes
    
    is_module_launch = "-m" in command
    if not is_module_launch:
        script_path = command[-1]
        if not os.path.exists(script_path):
            return False, f"[{log_prefix}] ERROR: Файл '{script_path}' не найден."

    env = os.environ.copy()
    env["PYTHONIOENCODING"] = "utf-8"
    
    log_messages = []
    creation_flags = subprocess.CREATE_NO_WINDOW if sys.platform == "win32" else 0
    process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, encoding='utf-8', errors='ignore', creationflags=creation_flags, env=env)
    tool_processes.append(process)
    log_messages.append(f"[{log_prefix}] Сервер запущен (PID: {process.pid})")
    
    initial_signal_ok = False
    start_time = time.time()
    timeout = 60

    while time.time() - start_time < timeout:
        if process.poll() is not None:
            log_messages.append(f"[{log_prefix}] CRITICAL ERROR: Процесс неожиданно завершился."); 
            try:
                remaining_output = process.stdout.read()
                if remaining_output: log_messages.append("--- ВЫВОД ПРОЦЕССА ПЕРЕД ПАДЕНИЕМ ---"); log_messages.append(remaining_output); log_messages.append("------------------------------------")
            except: pass
            break
        
        line = process.stdout.readline().strip()
        if line:
            print(f"[{log_prefix}] {line}"); log_messages.append(f"[{log_prefix}] {line}")
            if ready_signal in line: initial_signal_ok = True; break
        else: time.sleep(0.1)

    if not initial_signal_ok:
        log_messages.append(f"[{log_prefix}] ERROR: Не дождались сигнала готовности '{ready_signal}' за {timeout} секунд.")
        return False, log_messages
        
    threading.Thread(target=lambda: [print(f"[{log_prefix}] {l.strip()}") for l in iter(process.stdout.readline, '') if l], daemon=True).start()
    return True, log_messages

def stop_all_tool_servers():
    # ... (код без изменений)
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
    main_window.withdraw()
    
    servers_to_start = {
        # ... (список серверов без изменений)
        "Playwright": ([sys.executable, "-m", "playwright_mcp"], "Uvicorn running"),
        "WebSearch": ([sys.executable, "-m", "mcp_searxng"], "Uvicorn running"),
        "TextEditor": ([sys.executable, "-m", "mcp_text_editor"], "Uvicorn running"),
        "FileScope": ([sys.executable, "-m", "filescopemcp"], "Uvicorn running"),
        "LSPServer": ([sys.executable, "-m", "mcp_language_server"], "Uvicorn running on http://127.0.0.1:8009"),
        "CodeSandbox": ([sys.executable, "-m", "mcp_code_runner.runner"], "Uvicorn running on http://127.0.0.1:8010"),
        "ChromaDB": ([sys.executable, "-m", "chroma_mcp"], "Uvicorn running"),
        
        "AshraLauncher": ([sys.executable, "-u", "mcp_servers/mcp_ashra_server.py"], "MCP_ASHRA_READY"),
        "RAGLauncher": ([sys.executable, "-u", "mcp_servers/mcp_rag_server.py"], "MCP_RAG_READY"),
        "FeedbackServer": ([sys.executable, "-u", "mcp_servers/mcp_feedback_server.py"], "MCP_FEEDBACK_READY"),
        "FileSystem": ([sys.executable, "-u", "mcp_servers/mcp_file_server.py"], "MCP_FILE_SYSTEM_READY"),
        "GitHub": ([sys.executable, "-u", "mcp_servers/mcp_github_server.py"], "MCP_GITHUB_READY"),
        "GitLab": ([sys.executable, "-u", "mcp_servers/mcp_gitlab_server.py"], "MCP_GITLAB_READY"),
    }

    all_servers_ok = True
    for name, (command, signal) in servers_to_start.items():
        print(f"[Launcher] Запускаю сервер '{name}'...")
        current_env = os.environ.copy()
        current_env["PYTHONIOENCODING"] = "utf-8"
        
        success, logs = start_tool_server(command, name, signal)
        if not success:
            logs_str = "\n".join(logs)
            messagebox.showerror("Критическая ошибка запуска", f"Не удалось запустить сервер '{name}'.\n\nПолный лог:\n\n{logs_str}")
            all_servers_ok = False
            break
    
    if not all_servers_ok:
        stop_all_tool_servers()
        sys.exit(1)

    main_window.deiconify()
    
    engine = OrchestratorEngine(log_callback=lambda msg: print(f"[Engine Log] {msg}"))
    
    try:
        voice_controller = VoiceController(engine)
        engine.set_voice_controller(voice_controller)
        # УБРАН АВТОМАТИЧЕСКИЙ ЗАПУСК: voice_controller.start()
    except Exception as e:
        messagebox.showerror("Ошибка запуска VoiceEngine", f"Не удалось запустить голосовой движок: {e}\n\nПрограмма продолжит работу без голосового управления.")
        print(f"[Launcher] [CRITICAL] Ошибка инициализации VoiceController: {e}")

    app = AppUI(main_window, engine)
    
    main_window.mainloop()