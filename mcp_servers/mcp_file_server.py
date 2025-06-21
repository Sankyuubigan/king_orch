# mcp_servers/mcp_file_server.py - НОВЫЙ ИНСТРУМЕНТ ДЛЯ РАБОТЫ С ФАЙЛАМИ

import uvicorn
from fastapi import FastAPI
from fastapi.responses import JSONResponse
from pydantic import BaseModel
import os
from contextlib import asynccontextmanager

# Создаем рабочую директорию для агента, если ее нет
WORKSPACE_DIR = "workspace"
if not os.path.exists(WORKSPACE_DIR):
    os.makedirs(WORKSPACE_DIR)

def log_message(message):
    print(message, flush=True)

@asynccontextmanager
async def lifespan(app: FastAPI):
    log_message("[FileSystem] Сервер для работы с файлами запущен.")
    log_message(f"[FileSystem] Рабочая директория: {os.path.abspath(WORKSPACE_DIR)}")
    log_message("[FileSystem] MCP_FILE_SYSTEM_READY")
    yield
    log_message("[FileSystem] Сервер остановлен.")

app = FastAPI(lifespan=lifespan)

class FileContent(BaseModel):
    path: str
    content: str

def _safe_path(path: str) -> str:
    """Предотвращает выход за пределы рабочей директории."""
    full_path = os.path.abspath(os.path.join(WORKSPACE_DIR, path))
    if not full_path.startswith(os.path.abspath(WORKSPACE_DIR)):
        raise ValueError("Доступ за пределами рабочей директории запрещен.")
    return full_path

@app.post("/write_file")
async def write_file(item: FileContent):
    try:
        path = _safe_path(item.path)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, 'w', encoding='utf-8') as f:
            f.write(item.content)
        log_message(f"[FileSystem] Файл записан: {item.path}")
        return JSONResponse({"result": f"Файл '{item.path}' успешно записан."})
    except Exception as e:
        return JSONResponse(status_code=500, content={"result": f"Ошибка записи файла: {e}"})

@app.get("/read_file")
async def read_file(path: str):
    try:
        full_path = _safe_path(path)
        if not os.path.exists(full_path):
            return JSONResponse(status_code=404, content={"result": f"Файл не найден: {path}"})
        with open(full_path, 'r', encoding='utf-8') as f:
            content = f.read()
        log_message(f"[FileSystem] Файл прочитан: {path}")
        return JSONResponse({"result": content})
    except Exception as e:
        return JSONResponse(status_code=500, content={"result": f"Ошибка чтения файла: {e}"})

@app.get("/list_files")
async def list_files(path: str = "."):
    try:
        full_path = _safe_path(path)
        files = os.listdir(full_path)
        log_message(f"[FileSystem] Запрошен список файлов в: {path}")
        return JSONResponse({"result": "\n".join(files)})
    except Exception as e:
        return JSONResponse(status_code=500, content={"result": f"Ошибка листинга файлов: {e}"})

if __name__ == "__main__":
    uvicorn.run(app, host="127.0.0.1", port=7780, log_level="warning")