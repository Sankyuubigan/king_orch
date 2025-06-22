# mcp_servers/mcp_feedback_server.py - НОВЫЙ СЕРВЕР ДЛЯ ОБРАТНОЙ СВЯЗИ С ЧЕЛОВЕКОМ

import uvicorn
from fastapi import FastAPI, Request
from fastapi.responses import JSONResponse
from contextlib import asynccontextmanager
import asyncio

# Этот объект будет использоваться для "проброса" запроса в UI
feedback_request = {"event": asyncio.Event(), "question": None, "answer": None}

def log_message(message):
    print(message, flush=True)

@asynccontextmanager
async def lifespan(app: FastAPI):
    log_message("[FeedbackServer] Сервер для обратной связи с пользователем запущен.")
    log_message("[FeedbackServer] MCP_FEEDBACK_READY")
    yield
    log_message("[FeedbackServer] Сервер остановлен.")

app = FastAPI(lifespan=lifespan)

@app.post("/ask")
async def ask_user(request: Request):
    """Агент вызывает этот эндпоинт, чтобы задать вопрос."""
    body = await request.json()
    question = body.get("prompt")
    if not question:
        return JSONResponse(status_code=400, content={"error": "Параметр 'prompt' обязателен"})
    
    log_message(f"[FeedbackServer] Получен вопрос от агента: {question}")
    
    # Устанавливаем вопрос и сбрасываем событие
    feedback_request["question"] = question
    feedback_request["answer"] = None
    feedback_request["event"].clear()
    
    # Ждем, пока UI-поток установит ответ и вызовет .set()
    try:
        await asyncio.wait_for(feedback_request["event"].wait(), timeout=300.0)
    except asyncio.TimeoutError:
        return JSONResponse(status_code=504, content={"result": "Пользователь не ответил вовремя."})
        
    log_message(f"[FeedbackServer] Получен ответ от пользователя: {feedback_request['answer']}")
    return JSONResponse(status_code=200, content={"result": feedback_request["answer"]})

@app.get("/get_question")
async def get_question():
    """UI вызывает этот эндпоинт, чтобы проверить, есть ли вопрос от агента."""
    if feedback_request["question"]:
        return JSONResponse(status_code=200, content={"question": feedback_request["question"]})
    return JSONResponse(status_code=204) # No Content

@app.post("/provide_answer")
async def provide_answer(request: Request):
    """UI вызывает этот эндпоинт, чтобы предоставить ответ."""
    body = await request.json()
    answer = body.get("answer")
    
    feedback_request["answer"] = answer
    feedback_request["question"] = None # Сбрасываем вопрос
    feedback_request["event"].set() # Сигнализируем, что ответ получен
    
    return JSONResponse(status_code=200, content={"status": "ok"})

if __name__ == "__main__":
    uvicorn.run(app, host="127.0.0.1", port=7787, log_level="warning")