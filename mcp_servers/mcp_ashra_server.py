# mcp_servers/mcp_ashra_server.py - УЛУЧШЕННЫЙ ЗАПУСКАТОР ASHRA

import uvicorn
from fastapi import FastAPI
import subprocess
import os
import sys
from contextlib import asynccontextmanager

# --- БЛОК ПРОВЕРКИ ЗАВИСИМОСТЕЙ ---
try:
    import ashra
except ImportError:
    print("="*80)
    print("!!! КРИТИЧЕСКАЯ ОШИБКА: ПАКЕТ 'ashra-mcp' НЕ НАЙДЕН !!!")
    print("Пожалуйста, установите его, выполнив в терминале команду:")
    print("pip install ashra-mcp")
    print("="*80)
    sys.exit(1)
# --- КОНЕЦ БЛОКА ПРОВЕРКИ ---


def log_message(message):
    print(message, flush=True)

@asynccontextmanager
async def lifespan(app: FastAPI):
    log_message("[AshraLauncher] Сервер-запускатор для Ashra запущен.")
    
    # Запускаем сам сервер ashra как подпроцесс
    # Он будет работать на порту 8000 по умолчанию
    env = os.environ.copy()
    env["PYTHONIOENCODING"] = "utf-8"
    
    # Находим путь к исполняемому файлу ashra
    ashra_command = [sys.executable, "-m", "ashra"]
    
    ashra_process = subprocess.Popen(
        ashra_command,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        encoding='utf-8',
        errors='ignore',
        env=env
    )
    app.state.ashra_process = ashra_process
    log_message(f"[AshraLauncher] Процесс Ashra MCP запущен (PID: {ashra_process.pid}) на порту 8000.")
    
    log_message("[AshraLauncher] MCP_ASHRA_READY")
    yield
    log_message("[AshraLauncher] Остановка процесса Ashra MCP...")
    app.state.ashra_process.terminate()
    try:
        app.state.ashra_process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        app.state.ashra_process.kill()
    log_message("[AshraLauncher] Сервер-запускатор для Ashra остановлен.")

app = FastAPI(lifespan=lifespan)

@app.get("/")
async def root():
    return {"message": "Ashra Launcher is running. Interact with Ashra on port 8000."}

if __name__ == "__main__":
    uvicorn.run(app, host="127.0.0.1", port=7785, log_level="warning")