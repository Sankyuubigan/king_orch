# main.py - Добавлено создание новых директорий

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

def create_dirs():
    # --- ИЗМЕНЕНО: Добавлено создание папок для новой структуры ---
    os.makedirs("agents", exist_ok=True); os.makedirs("crews", exist_ok=True)
    os.makedirs("mcp_servers", exist_ok=True); os.makedirs("prompts", exist_ok=True)
    os.makedirs("utils", exist_ok=True); os.makedirs("tools", exist_ok=True)
    os.makedirs("voice_engine/stt", exist_ok=True) # <-- STT
    os.makedirs("voice_engine/tts/silero", exist_ok=True) # <-- TTS
    os.makedirs("voice_engine/silero_cache", exist_ok=True)
    os.makedirs(VENDOR_DIR, exist_ok=True)
    for d in ["agents", "crews", "utils", "voice_engine"]:
        init_file = os.path.join(d, "__init__.py")
        if not os.path.exists(init_file):
            with open(init_file, "a"): pass

def is_port_in_use(port: int) -> bool:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        return s.connect_ex(('127.0.0.1', port)) == 0

def get_port_from_command(command: list) -> int | None:
    for arg in command:
        match = re.search(r'(?:--port(?:=|\s)|:)(\d+)', arg)
        if match: return int(match.group(1))
    return None

def background_startup_tasks(app_ui: AppUI):
    servers_to_start = {
        "Playwright": { "cmd": ["npx", "--yes", "@playwright/mcp@latest", "--port=7800"], "signal": "Listening on", "cwd": None },
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
    
    for name, config in servers_to_start.items():
        if config.get("cwd") and not os.path.isdir(config["cwd"]):
            print(f"[Launcher] [INFO] Директория для '{name}' не найдена: {config['cwd']}. Сервер пропускается.")
            continue
        port = get_port_from_command(config["cmd"])
        if port and is_port_in_use(port):
            print(f"[Launcher] [WARNING] Порт {port} для сервера '{name}' уже занят. Пропускаю запуск, предполагая, что он уже работает.")
            continue
        print(f"[Launcher] Запускаю сервер '{name}'...")
        start_tool_server(config["cmd"], name, config["signal"], cwd=config.get("cwd"))

    build_active_tools.generate_active_tools_config()
    
    engine = OrchestratorEngine(log_callback=lambda msg: print(f"[Engine Log] {msg}"))
    
    try:
        voice_controller = VoiceController(engine)
        engine.set_voice_controller(voice_controller)
    except Exception as e:
        print("\n" + "="*80)
        print("!!! ПРЕДУПРЕЖДЕНИЕ: ОШИБКА VOICEENGINE !!!")
        print(f"Не удалось запустить голосовой движок: {e}")
        print("Проверьте, что модели Vosk и Silero скачаны, а также выбраны в настройках (кнопка ⚙️).")
        print("Программа продолжит работу без голосового управления.")
        print("="*80 + "\n")
        
    app_ui.set_engine(engine)

def start_tool_server(command, log_prefix, ready_signal, cwd=None):
    global tool_processes
    env = os.environ.copy()
    env["PYTHONIOENCODING"] = "utf-8"
    if cwd: env["PYTHONPATH"] = os.path.abspath(cwd) + os.pathsep + env.get("PYTHONPATH", "")
    creation_flags = subprocess.CREATE_NO_WINDOW if sys.platform == "win32" else 0
    use_shell = sys.platform == "win32" and command[0] in ["node", "npx"]
    process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, encoding='utf-8', errors='ignore', creationflags=creation_flags, env=env, cwd=cwd, shell=use_shell)
    tool_processes.append(process)
    output_thread = threading.Thread(target=lambda: [print(f"[{log_prefix}] {l.strip()}") for l in iter(process.stdout.readline, '') if l], daemon=True)
    output_thread.start()
    return True

def stop_all_tool_servers():
    print("[Launcher] Остановка всех MCP-серверов...")
    for process in tool_processes:
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