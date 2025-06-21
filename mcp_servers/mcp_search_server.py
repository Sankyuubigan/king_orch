# mcp_search_server.py - НОВЫЙ, ЛЕГКОВЕСНЫЙ СЕРВЕР ДЛЯ ПОИСКА ЧЕРЕЗ SEARXNG

import uvicorn
from fastapi import FastAPI, Request
from fastapi.responses import JSONResponse
import requests
import json
from contextlib import asynccontextmanager

# Инстанс SearXNG, который мы будем использовать
SEARXNG_INSTANCE = "https://searx.work"

def log_message(message):
    print(message, flush=True)

@asynccontextmanager
async def lifespan(app: FastAPI):
    log_message("[WebSearch] Сервер запущен.")
    log_message(f"[WebSearch] Используется инстанс: {SEARXNG_INSTANCE}")
    log_message("[WebSearch] MCP_SEARCH_READY")
    yield
    log_message("[WebSearch] Сервер остановлен.")

app = FastAPI(lifespan=lifespan)

@app.post("/v1/action")
async def perform_search(request: Request):
    try:
        body = await request.json()
        query = body.get("action", {}).get("goal")
        if not query:
            return JSONResponse(status_code=400, content={"error": "'goal' не найден"})

        log_message(f"[WebSearch] Поиск по запросу: {query}")

        # Параметры для запроса к SearXNG
        params = {
            'q': query,
            'format': 'json',
            'language': 'ru'
        }
        
        # Выполняем запрос без использования браузера
        response = requests.get(f"{SEARXNG_INSTANCE}/search", params=params, timeout=10)
        response.raise_for_status() # Проверяем на ошибки HTTP
        
        data = response.json()
        
        # Собираем чистый и структурированный результат для модели
        results_text = f"Результаты поиска по запросу '{query}':\n\n"
        for item in data.get("results", [])[:5]: # Берем топ-5 результатов
            title = item.get('title', 'Без заголовка')
            content = item.get('content', 'Нет описания.')
            url = item.get('url', '')
            results_text += f"### {title}\n"
            results_text += f"**Источник:** {url}\n"
            results_text += f"**Содержание:** {content}\n\n"
        
        if not data.get("results"):
            results_text = "По вашему запросу ничего не найдено."

        return JSONResponse(status_code=200, content={"result": results_text})

    except Exception as e:
        error_message = f"Ошибка при выполнении поиска: {type(e).__name__} - {e}"
        log_message(f"[WebSearch] {error_message}")
        return JSONResponse(status_code=200, content={"result": error_message})

if __name__ == "__main__":
    uvicorn.run(app, host="127.0.0.1", port=7778, log_level="warning")