# mcp_servers/mcp_github_server.py - НОВЫЙ СЕРВЕР-ЗАГЛУШКА ДЛЯ GITHUB

import uvicorn
from fastapi import FastAPI, Request
from fastapi.responses import JSONResponse
from contextlib import asynccontextmanager

# !!! ВАЖНО ДЛЯ СНЯТИЯ ЛИМИТОВ !!!
# Чтобы избежать низких лимитов на запросы, необходимо использовать токен.
# Получите его в настройках своего GitHub-аккаунта (Developer settings -> Personal access tokens)
# и вставьте сюда.
GITHUB_API_TOKEN = None # Например, "ghp_xxxxxxxxxxxxxxxxxxxx"

def log_message(message):
    print(message, flush=True)

@asynccontextmanager
async def lifespan(app: FastAPI):
    log_message("[GitHub] Сервер для поиска по GitHub запущен.")
    if GITHUB_API_TOKEN:
        log_message("[GitHub] Обнаружен API токен. Запросы будут аутентифицированы.")
    else:
        log_message("[GitHub] [WARNING] API токен не предоставлен. Возможны низкие лимиты на запросы.")
    log_message("[GitHub] MCP_GITHUB_READY")
    yield
    log_message("[GitHub] Сервер остановлен.")

app = FastAPI(lifespan=lifespan)

@app.post("/v1/action")
async def search_github(request: Request):
    try:
        body = await request.json()
        query = body.get("action", {}).get("goal")
        if not query:
            return JSONResponse(status_code=400, content={"error": "'goal' не найден"})

        log_message(f"[GitHub] Поиск репозиториев по запросу: {query}")

        # Здесь должна быть реальная логика обращения к GitHub API v3
        # (например, через библиотеку requests или PyGithub)
        # GET https://api.github.com/search/repositories?q={query}
        # Headers: {"Authorization": f"token {GITHUB_API_TOKEN}"}
        
        # --- НАЧАЛО ЗАГЛУШКИ ---
        # Имитируем успешный ответ от API
        mock_results = {
            "total_count": 2,
            "items": [
                {
                    "full_name": f"mockuser/{query.replace(' ', '-')}-project",
                    "description": "Это пример репозитория, найденного по вашему запросу.",
                    "html_url": f"https://github.com/mockuser/{query.replace(' ', '-')}-project",
                    "stargazers_count": 123,
                    "language": "Python"
                },
                {
                    "full_name": "awesome-org/awesome-tool",
                    "description": "Еще один релевантный проект.",
                    "html_url": "https://github.com/awesome-org/awesome-tool",
                    "stargazers_count": 456,
                    "language": "Go"
                }
            ]
        }
        
        results_text = f"Результаты поиска по GitHub для '{query}':\n\n"
        for item in mock_results.get("items", []):
            results_text += f"### {item.get('full_name')}\n"
            results_text += f"**URL:** {item.get('html_url')}\n"
            results_text += f"**Описание:** {item.get('description')}\n"
            results_text += f"**Звезд:** {item.get('stargazers_count')}\n\n"
        # --- КОНЕЦ ЗАГЛУШКИ ---

        return JSONResponse(status_code=200, content={"result": results_text})

    except Exception as e:
        error_message = f"Ошибка при поиске в GitHub: {e}"
        log_message(f"[GitHub] {error_message}")
        return JSONResponse(status_code=200, content={"result": error_message})

if __name__ == "__main__":
    uvicorn.run(app, host="127.0.0.1", port=7782, log_level="warning")