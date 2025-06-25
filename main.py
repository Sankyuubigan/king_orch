# main.py - ЗАПУСК ИНСТРУМЕНТОВ ИЗ ИЗОЛИРОВАННЫХ ОКРУЖЕНИЙ

import tkinter as tk
import subprocess
import atexit
import sys
import os
import threading
import time
import re
import socket

from ui import AppUI
from engine import OrchestratorEngine
from voice_engine.controller import VoiceController
import build_active_tools

tool_processes = []
VENDOR_DIR = "vendor"
# --- ИСПРАВЛЕНО: Пути к портативным средам сделаны абсолютными ---
NODE_DIR = os.path.join(VENDOR_DIR, "nodejs")
NODE_EXE = os.path.abspath(os.path.join(NODE_DIR, "node.exe"))
NPX_CMD = os.path.abspath(os.path.join(NODE_DIR, "npx.cmd"))

PYTHON_DIR = os.path.join(VENDOR_DIR, "python")
PYTHON_EXE = os.path.abspath(os.path.join(PYTHON_DIR, "python.exe"))

def get_venv_python(tool_name: str) -> str:
    """Возвращает АБСОЛЮТНЫЙ путь к python.exe в виртуальном окружении инструмента."""
    # ИСПРАВЛЕНО: Путь сделан абсолютным
    return os.path.abspath(os.path.join(VENDOR_DIR, tool_name, ".venv", "Scripts", "python.exe"))

def create_dirs():
    # ... (код без изменений)
    os.makedirs("agents", exist_ok=True); os.makedirs("crews", exist_ok=True)
    os.makedirs("mcp_servers", exist_ok=True); os.makedirs("prompts", exist_ok=True)
    os.makedirs("utils", exist_ok=True); os.makedirs("tools", exist_ok=True)
    os.makedirs("voice_engine/stt", exist_ok=True)
    os.makedirs("voice_engine/tts/silero", exist_ok=True)
    os.makedirs("voice_engine/silero_cache", exist_ok=True)
    os.makedirs(VENDOR_DIR, exist_ok=True)
    os.makedirs("workspace", exist_ok=True)
    for d in ["agents", "crews", "utils", "voice_engine", "mcp_servers"]:
        init_file = os.path.join(d, "__init__.py")
        if not os.path.exists(init_file):
            with open(init_file, "a"): pass

def is_port_in_use(port: int) -> bool:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        return s.connect_ex(('127.0.0.1', port)) == 0

def get_port_from_command(command: list) -> int | None:
    for i, arg in enumerate(command):
        if arg == '--port' and i + 1 < len(command):
            return int(command[i+1])
        if arg.startswith('--port='):
            return int(arg.split('=', 1)[1])
    return None

def background_startup_tasks(app_ui: AppUI):
    # Команды запуска теперь используют абсолютные пути, что делает их надежными
    servers_to_start = {
        "Playwright": { "cmd": [NPX_CMD, "--yes", "@playwright/mcp@latest", "--port", "7800"], "signal": "Listening on" },
        "WebSearch": { "cmd": [NODE_EXE, "dist/main.js"], "signal": "Server is running on port", "cwd": os.path.join(VENDOR_DIR, "mcp-searxng") },
        "RAG": { "cmd": [NODE_EXE, "dist/main.js"], "signal": "Server is running on port 8001", "cwd": os.path.join(VENDOR_DIR, "rag-mcp") },
        
        "Chroma": { "cmd": [get_venv_python("chroma-mcp"), "-u", "-m", "chroma_mcp"], "signal": "Uvicorn running" },
        "CodeSandbox": { "cmd": [get_venv_python("mcp-code-runner"), "-u", "runner.py"], "signal": "Uvicorn running", "cwd": os.path.join(VENDOR_DIR, "mcp-code-runner") },
        "LSPServer": { "cmd": [get_venv_python("mcp-language-server"), "-u", "main.py"], "signal": "Uvicorn running", "cwd": os.path.join(VENDOR_DIR, "mcp-language-server") },
        "MarkItDown": { "cmd": [PYTHON_EXE, "-m", "markitdown_mcp", "--http", "--port", "7790"], "signal": "Serving MarkItDown" },

        "FeedbackServer": { "cmd": [PYTHON_EXE, "-u", "mcp_servers/mcp_feedback_server.py"], "signal": "MCP_FEEDBACK_READY" },
        "FileSystem": { "cmd": [PYTHON_EXE, "-u", "mcp_servers/mcp_file_server.py"], "signal": "MCP_FILE_SYSTEM_READY" },
        "Indexer": { "cmd": [PYTHON_EXE, "-u", "mcp_servers/mcp_indexer_server.py"], "signal": "MCP_INDEXER_READY" },
    }
    
    for name, config in servers_to_start.items():
        if not os.path.exists(config["cmd"][0]):
            print(f"[Launcher] [ERROR] Исполняемый файл для '{name}' не найден: {config['cmd'][0]}. Сервер пропускается.")
            continue
        if config.get("cwd") and not os.path.isdir(config["cwd"]):
            print(f"[Launcher] [INFO] Директория для '{name}' не найдена: {config['cwd']}. Сервер пропускается.")
            continue
        port = get_port_from_command(config["cmd"])
        if port and is_port_in_use(port):
            print(f"[Launcher] [WARNING] Порт {port} для сервера '{name}' уже занят. Пропускаю запуск.")
            continue
        print(f"[Launcher] Запускаю сервер '{name}'...")
        start_tool_server(config["cmd"], name, config.get("signal"), cwd=config.get("cwd"))

    print("[Launcher] Ожидание запуска серверов перед сборкой конфига...")
    time.sleep(5)
    build_active_tools.generate_active_tools_config()
    
    engine = OrchestratorEngine(log_callback=lambda msg: print(f"[Engine Log] {msg}"))
    
    try:
        voice_controller = VoiceController(engine)
        engine.set_voice_controller(voice_controller)
    except Exception as e:
        print(f"\n[Launcher] [WARNING] Не удалось запустить голосовой движок: {e}")
        
    app_ui.set_engine(engine)

def start_tool_server(command, log_prefix, ready_signal, cwd=None):
    # ... (код без изменений)
    global tool_processes
    env = os.environ.copy()
    env["PYTHONIOENCODING"] = "utf-8"
    if cwd: env["PYTHONPATH"] = os.path.abspath(cwd) + os.pathsep + env.get("PYTHONPATH", "")
    
    creation_flags = subprocess.CREATE_NO_WINDOW if sys.platform == "win32" else 0
    use_shell = sys.platform == "win32"
    
    process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, encoding='utf-8', errors='ignore', creationflags=creation_flags, env=env, cwd=cwd, shell=use_shell)
    tool_processes.append(process)
    
    def log_output():
        for line in iter(process.stdout.readline, ''):
            if line: print(f"[{log_prefix}] {line.strip()}")

    output_thread = threading.Thread(target=log_output, daemon=True)
    output_thread.start()
    return True

def stop_all_tool_servers():
    # ... (код без изменений)
    print("[Launcher] Остановка всех MCP-серверов...")
    for process in reversed(tool_processes):
        if process.poll() is None:
            try:
                if sys.platform == "win32":
                    subprocess.run(['taskkill', '/F', '/T', '/PID', str(process.pid)], check=False, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                else: process.terminate()
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
    app = AppUI(main_window, None)
    startup_thread = threading.Thread(target=background_startup_tasks, args=(app,), daemon=True)
    startup_thread.start()
    main_window.mainloop()