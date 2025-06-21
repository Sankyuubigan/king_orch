# mcp_browser_server.py - ЧИСТАЯ ВЕРСИЯ. ТОЛЬКО УПРАВЛЕНИЕ БРАУЗЕРОМ.

import asyncio
import uvicorn
from fastapi import FastAPI, Request, Response
from fastapi.responses import JSONResponse
from playwright.async_api import async_playwright
import sys
import re
from contextlib import asynccontextmanager

playwright_context = {}

def log_message(message):
    print(message, flush=True)

@asynccontextmanager
async def lifespan(app: FastAPI):
    log_message("[BrowserAgent] Запуск браузера...")
    try:
        playwright = await async_playwright().start()
        browser = await playwright.chromium.launch(headless=True)
        context = await browser.new_context(
            user_agent="Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36"
        )
        page = await context.new_page()
        
        playwright_context.update({"playwright": playwright, "browser": browser, "page": page})
        
        log_message("[BrowserAgent] Браузер и страница готовы.")
        log_message("[BrowserAgent] MCP_SERVER_READY")
        yield
    finally:
        browser = playwright_context.get("browser")
        playwright = playwright_context.get("playwright")
        if browser: await browser.close()
        if playwright: await playwright.stop()
        log_message("[BrowserAgent] Браузер остановлен.")

app = FastAPI(lifespan=lifespan)

@app.get("/screenshot", response_class=Response)
async def get_screenshot():
    page = playwright_context.get("page")
    if not page or page.is_closed():
        return JSONResponse(status_code=503, content={"error": "Страница не инициализирована."})
    return Response(content=await page.screenshot(), media_type="image/png")

def parse_command(goal: str):
    match = re.match(r"(\w+)\((.*)\)", goal, re.DOTALL)
    if not match: return None, None
    command, args_str = match.groups()
    kwargs = {}
    if args_str:
        try: kwargs = eval(f"dict({args_str})", {"dict": dict})
        except Exception as e:
            log_message(f"[Parser] Ошибка парсинга: {e}")
            return command, {}
    return command, kwargs

@app.post("/v1/action")
async def perform_action(request: Request):
    page = playwright_context.get("page")
    if not page: return JSONResponse(status_code=503, content={"error": "Браузер не готов."})

    try:
        body = await request.json()
        goal = body.get("action", {}).get("goal")
        if not goal: return JSONResponse(status_code=400, content={"error": "'goal' не найден"})

        log_message(f"[BrowserAgent] Получена команда: {goal}")
        command, kwargs = parse_command(goal)

        if command == "goto":
            url = kwargs.get('url', '')
            if not url.startswith('http'): url = f"https://{url}"
            await page.goto(url, wait_until="domcontentloaded", timeout=30000)
        elif command == "click":
            await page.click(**kwargs)
        elif command == "type":
            selector = kwargs.pop('selector')
            await page.locator(selector).type(**kwargs)
        else:
            # Если команда не распознана, возвращаем ошибку
            return JSONResponse(status_code=400, content={"result": f"Неизвестная команда для браузера: {command}"})

        await asyncio.sleep(1.5)
        content = await page.evaluate("document.body.innerText")
        log_message("[BrowserAgent] Команда выполнена.")
        return JSONResponse(status_code=200, content={"result": content[:8000]})

    except Exception as e:
        error_message = f"Ошибка выполнения команды: {type(e).__name__} - {e}"
        log_message(f"[BrowserAgent] {error_message}")
        return JSONResponse(status_code=200, content={"result": error_message})

if __name__ == "__main__":
    uvicorn.run(app, host="127.0.0.1", port=7777, log_level="warning")