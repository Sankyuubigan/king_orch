# mcp_servers/mcp_fetcher_server.py - ИСПРАВЛЕННАЯ ВЕРСИЯ С ДОБАВЛЕННЫМ ИМПОРТОМ

import asyncio
import uvicorn
from fastapi import FastAPI, Request # <--- ВОТ ИСПРАВЛЕНИЕ
from fastapi.responses import JSONResponse
from playwright.async_api import async_playwright
from contextlib import asynccontextmanager

playwright_context = {}

def log_message(message):
    print(message, flush=True)

@asynccontextmanager
async def lifespan(app: FastAPI):
    log_message("[Fetcher] Запуск 'добытчика' контента...")
    try:
        playwright = await async_playwright().start()
        browser = await playwright.chromium.launch(headless=True)
        playwright_context.update({"playwright": playwright, "browser": browser})
        log_message("[Fetcher] 'Добытчик' готов.")
        log_message("[Fetcher] MCP_FETCHER_READY")
        yield
    finally:
        browser = playwright_context.get("browser")
        playwright = playwright_context.get("playwright")
        if browser: await browser.close()
        if playwright: await playwright.stop()
        log_message("[Fetcher] 'Добытчик' остановлен.")

app = FastAPI(lifespan=lifespan)

@app.post("/v1/action")
async def perform_fetch(request: Request): # Теперь Python знает, что такое Request
    browser = playwright_context.get("browser")
    if not browser: return JSONResponse(status_code=503, content={"error": "Браузер не готов."})
    
    try:
        body = await request.json()
        url = body.get("action", {}).get("goal")
        if not url or not url.startswith('http'):
            return JSONResponse(status_code=400, content={"result": f"Получен некорректный URL: {url}"})

        log_message(f"[Fetcher] Извлекаю контент с: {url}")
        
        context = await browser.new_context(user_agent="Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36")
        page = await context.new_page()
        
        await page.goto(url, wait_until="domcontentloaded", timeout=45000)
        await asyncio.sleep(2)
        
        content = await page.evaluate("document.body.innerText")
        
        await context.close()
        
        log_message(f"[Fetcher] Контент извлечен, {len(content)} символов.")
        return JSONResponse(status_code=200, content={"result": content[:15000]})

    except Exception as e:
        error_message = f"Ошибка при извлечении контента: {type(e).__name__} - {e}"
        log_message(f"[Fetcher] {error_message}")
        return JSONResponse(status_code=200, content={"result": error_message})

if __name__ == "__main__":
    uvicorn.run(app, host="127.0.0.1", port=7779, log_level="warning")