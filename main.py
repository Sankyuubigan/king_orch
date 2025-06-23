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
VENDOR_DIR = "vendor"

def create_dirs():
    os.makedirs("agents", exist_ok=True); os.makedirs("crews", exist_ok=True)
    os.makedirs("mcp_servers", exist_ok=True); os.makedirs("prompts", exist_ok=True)
    os.makedirs("utils", exist_ok=True); os.makedirs("tools", exist_ok=True)
    os.makedirs("voice_engine/vosk", exist_ok=True); os.makedirs("voice_engine/silero_cache", exist_ok=True)
    os.makedirs(VENDOR_DIR, exist_ok=True)
    for d in ["agents", "crews", "utils", "voice_engine"]:
        init_file = os.path.join(d, "__init__.py")
        if not os.path.exists(init_file):
            with open(init_file, "a"): pass

def start_tool_server(command, log_prefix, ready_signal, cwd=None):
    global tool_processes
    env = os.environ.copy()
    env["PYTHONIOENCODING"] = "utf-8"
    if cwd:
        env["PYTHONPATH"] = os.path.abspath(cwd) + os.pathsep + env.get("PYTHONPATH", "")

    log_messages = []
    creation_flags = subprocess.CREATE_NO_WINDOW if sys.platform == "win32" else 0
    use_shell = sys.platform == "win32" and command[0] == "node"
    
    process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, encoding='utf-8', errors='ignore', creationflags=creation_flags, env=env, cwd=cwd, shell=use_shell)
    tool_processes.append(process)
    log_messages.append(f"[{log_prefix}] Сервер запущен (PID: {process.pid})")
    
    initial_signal_ok = False
    start_time = time.time()
    timeout = 45

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
        "Playwright": { "cmd": [sys.executable, "-m", "mcp_server_playwright.server"], "signal": "Uvicorn running on http://127.0.0.1:7800", "cwd": os.path.join(VENDOR_DIR, "mcp-server-playwright") },
        "WebSearch": { "cmd": [sys.executable, "main.py"], "signal": "Uvicorn running", "cwd": os.path.join(VENDOR_DIR, "mcp-searxng") },
        "TextEditor": { "cmd": [sys.executable, "-m", "mcp_text_editor.main"], "signal": "Uvicorn running", "cwd": os.path.join(VENDOR_DIR, "mcp-text-editor") },
        "FileScope": { "cmd": [sys.executable, "-m", "filescopemcp"], "signal": "Uvicorn running", "cwd": os.path.join(VENDOR_DIR, "FileScopeMCP") },
        "LSPServer": { "cmd": [sys.executable, "main.py"], "signal": "Uvicorn running on http://127.0.0.1:8009", "cwd": os.path.join(VENDOR_DIR, "mcp-language-server") },
        "CodeSandbox": { "cmd": [sys.executable, "runner.py"], "signal": "Uvicorn running on http://127.0.0.1:8010", "cwd": os.path.join(VENDOR_DIR, "mcp-code-runner") },
        "Ashra": { "cmd": ["node", "build/index.js"], "signal": "Ashra MCP Server running on stdio", "cwd": os.path.join(VENDOR_DIR, "ashra-mcp") },
        "RAG": { "cmd": [sys.executable, "main.py"], "signal": "Uvicorn running on http://127.0.0.1:8001", "cwd": os.path.join(VENDOR_DIR, "rag-mcp") },
        "Chroma": { "cmd": [sys.executable, "-m", "chroma_mcp"], "signal": "Uvicorn running", "cwd": os.path.join(VENDOR_DIR, "chroma-mcp") },
        "FeedbackServer": { "cmd": [sys.executable, "-u", "mcp_servers/mcp_feedback_server.py"], "signal": "MCP_FEEDBACK_READY" },
        "FileSystem": { "cmd": [sys.executable, "-u", "mcp_servers/mcp_file_server.py"], "signal": "MCP_FILE_SYSTEM_READY" },
        "GitHub": { "cmd": [sys.executable, "-u", "mcp_servers/mcp_github_server.py"], "signal": "MCP_GITHUB_READY" },
        "GitLab": { "cmd": [sys.executable, "-u", "mcp_servers/mcp_gitlab_server.py"], "signal": "MCP_GITLAB_READY" },
    }

    failed_servers = []
    for name, config in servers_to_start.items():
        if config.get("cwd") and not os.path.isdir(config["cwd"]):
            print(f"[Launcher] [WARNING] Директория для '{name}' не найдена: {config['cwd']}. Сервер пропускается.")
            failed_servers.append(f"{name} (папка не найдена)")
            continue
        
        print(f"[Launcher] Запускаю сервер '{name}'...")
        success, logs = start_tool_server(config["cmd"], name, config["signal"], cwd=config.get("cwd"))
        if not success:
            failed_servers.append(name)
            print(f"[Launcher] [ERROR] Не удалось запустить сервер '{name}'.")
    
    if failed_servers:
        failed_list = "\n - ".join(failed_servers)
        messagebox.showwarning("Предупреждение при запуске", f"Не удалось запустить следующие MCP-серверы:\n - {failed_list}\n\nПриложение продолжит работу, но их функционал будет недоступен.")

    main_window.deiconify()
    
    engine = OrchestratorEngine(log_callback=lambda msg: print(f"[Engine Log] {msg}"))
    
    try:
        voice_controller = VoiceController(engine)
        engine.set_voice_controller(voice_controller)
    except Exception as e:
        messagebox.showwarning("Ошибка VoiceEngine", f"Не удалось запустить голосовой движок: {e}\n\nПрограмма продолжит работу без голосового управления.")

    app = AppUI(main_window, engine)
    
    main_window.mainloop()