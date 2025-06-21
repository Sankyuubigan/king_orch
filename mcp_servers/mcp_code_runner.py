# mcp_servers/mcp_code_runner.py - НОВЫЙ ИНСТРУМЕНТ ДЛЯ ЗАПУСКА КОДА

import uvicorn
from fastapi import FastAPI
from pydantic import BaseModel
import subprocess
import os
from contextlib import asynccontextmanager

WORKSPACE_DIR = "workspace"

def log_message(message):
    print(message, flush=True)

@asynccontextmanager
async def lifespan(app: FastAPI):
    log_message("[CodeRunner] Сервер для выполнения кода запущен.")
    log_message(f"[CodeRunner] Команды будут выполняться в директории: {os.path.abspath(WORKSPACE_DIR)}")
    log_message("[CodeRunner] MCP_CODE_RUNNER_READY")
    yield
    log_message("[CodeRunner] Сервер остановлен.")

app = FastAPI(lifespan=lifespan)

class Command(BaseModel):
    command: str

@app.post("/execute")
async def execute_command(item: Command):
    try:
        log_message(f"[CodeRunner] Выполняю команду: {item.command}")
        # TODO: Внедрить Docker для настоящей песочницы!
        # Сейчас команды выполняются напрямую, что НЕБЕЗОПАСНО.
        result = subprocess.run(
            item.command,
            shell=True,
            capture_output=True,
            text=True,
            cwd=WORKSPACE_DIR, # Выполняем команду в рабочей директории
            timeout=60
        )
        
        output = f"Exit Code: {result.returncode}\n"
        output += f"--- STDOUT ---\n{result.stdout}\n"
        output += f"--- STDERR ----\n{result.stderr}\n"
        
        log_message(f"[CodeRunner] Команда выполнена с кодом {result.returncode}")
        return {"result": output}

    except subprocess.TimeoutExpired:
        log_message("[CodeRunner] [ERROR] Команда выполнялась слишком долго.")
        return {"result": "Ошибка: Команда выполнялась слишком долго и была прервана."}
    except Exception as e:
        log_message(f"[CodeRunner] [ERROR] Ошибка выполнения команды: {e}")
        return {"result": f"Критическая ошибка выполнения команды: {e}"}

if __name__ == "__main__":
    uvicorn.run(app, host="127.0.0.1", port=7781, log_level="warning")