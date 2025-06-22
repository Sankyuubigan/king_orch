# mcp_servers/mcp_rag_server.py - ЗАПУСКАЕТ И УПРАВЛЯЕТ RAG-MCP

import uvicorn
from fastapi import FastAPI
import subprocess
import os
import sys
from contextlib import asynccontextmanager

RAG_PATH = "rag-mcp"
RAG_SCRIPT = "main.py"

def log_message(message):
    print(message, flush=True)

@asynccontextmanager
async def lifespan(app: FastAPI):
    log_message("[RAGLauncher] Сервер-запускатор для RAG запущен.")
    if not os.path.exists(os.path.join(RAG_PATH, RAG_SCRIPT)):
        log_message("="*80); log_message(f"!!! КРИТИЧЕСКАЯ ОШИБКА: НЕ НАЙДЕН СКРИПТ {RAG_SCRIPT} в папке {RAG_PATH} !!!"); log_message("Пожалуйста, выполните команду 'git clone https://github.com/hannesrudolph/rag-mcp.git'"); log_message("="*80); sys.exit(1)
        
    env = os.environ.copy()
    env["PYTHONIOENCODING"] = "utf-8"
    rag_process = subprocess.Popen([sys.executable, "-u", RAG_SCRIPT], cwd=RAG_PATH, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, encoding='utf-8', errors='ignore', env=env)
    app.state.rag_process = rag_process
    log_message(f"[RAGLauncher] Процесс RAG MCP запущен (PID: {rag_process.pid}) на порту 8001.")
    
    log_message("[RAGLauncher] MCP_RAG_READY")
    yield
    log_message("[RAGLauncher] Остановка процесса RAG MCP...")
    app.state.rag_process.terminate()
    try: app.state.rag_process.wait(timeout=5)
    except subprocess.TimeoutExpired: app.state.rag_process.kill()
    log_message("[RAGLauncher] Сервер-запускатор для RAG остановлен.")

app = FastAPI(lifespan=lifespan)

@app.get("/")
async def root():
    return {"message": "RAG Launcher is running. Interact with RAG on port 8001."}

if __name__ == "__main__":
    uvicorn.run(app, host="127.0.0.1", port=7786, log_level="warning")