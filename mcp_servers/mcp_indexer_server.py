# mcp_servers/mcp_indexer_server.py - НОВЫЙ СЕРВЕР ДЛЯ ИНДЕКСАЦИИ КОДА

import uvicorn
from fastapi import FastAPI
from fastapi.responses import JSONResponse
from contextlib import asynccontextmanager
import subprocess
import sys
import os

def log_message(message):
    print(message, flush=True)

@asynccontextmanager
async def lifespan(app: FastAPI):
    log_message("[IndexerServer] Сервер для индексации кода запущен.")
    log_message("[IndexerServer] MCP_INDEXER_READY")
    yield
    log_message("[IndexerServer] Сервер остановлен.")

app = FastAPI(lifespan=lifespan)

@app.post("/index")
async def index_project():
    """Запускает скрипт индексации и возвращает его вывод."""
    log_message("[IndexerServer] Получен запрос на индексацию проекта...")
    
    script_path = os.path.join("tools", "code_indexer.py")
    if not os.path.exists(script_path):
        log_message(f"[IndexerServer] [ERROR] Скрипт индексации не найден: {script_path}")
        return JSONResponse(status_code=500, content={"result": "Ошибка: скрипт индексации не найден."})

    try:
        # Запускаем индексатор как отдельный процесс, чтобы не блокировать сервер
        process = subprocess.run(
            [sys.executable, "-u", script_path],
            capture_output=True,
            text=True,
            encoding='utf-8',
            timeout=300 # 5 минут на индексацию
        )
        
        output = process.stdout + "\n" + process.stderr
        log_message("[IndexerServer] Индексация завершена.")
        
        if process.returncode == 0:
            return JSONResponse(status_code=200, content={"result": "Индексация успешно завершена.", "log": output})
        else:
            return JSONResponse(status_code=500, content={"result": "Ошибка в процессе индексации.", "log": output})

    except subprocess.TimeoutExpired:
        log_message("[IndexerServer] [ERROR] Время на индексацию истекло.")
        return JSONResponse(status_code=500, content={"result": "Ошибка: время на индексацию истекло."})
    except Exception as e:
        log_message(f"[IndexerServer] [ERROR] Критическая ошибка при запуске индексатора: {e}")
        return JSONResponse(status_code=500, content={"result": f"Критическая ошибка: {e}"})

if __name__ == "__main__":
    uvicorn.run(app, host="127.0.0.1", port=7788, log_level="warning")