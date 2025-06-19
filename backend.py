# backend.py - ФИНАЛЬНЫЙ, ПРОВЕРЕННЫЙ, СТАБИЛЬНЫЙ

import uvicorn
import subprocess
import os
import signal
import time
import asyncio
from fastapi import FastAPI, Request, HTTPException
from fastapi.staticfiles import StaticFiles
from fastapi.middleware.cors import CORSMiddleware
import logging

from copilotkit import CopilotKitSDK, LangGraphAgent
from copilotkit.integrations.fastapi import add_fastapi_endpoint
from langchain_ollama import OllamaLLM
from langchain_core.tools import tool
from langchain.prompts import PromptTemplate
from langchain.agents import AgentExecutor, create_react_agent

OLLAMA_PATH = r"D:\Projects\universal_orchestrator\ollama_runtime\ollama.exe"
DEFAULT_MODEL = "llama3:8b"
ollama_process = None
sdk: CopilotKitSDK = None

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)

# --- Инструменты (полный код) ---
@tool
def search_the_web(query: str) -> str:
    """Searches the web for the given query."""
    logger.info(f"🔎 Поиск в вебе: {query}")
    try:
        import requests
        url = f"https://api.duckduckgo.com/?q={query}&format=json&no_html=1"
        response = requests.get(url, timeout=10).json()
        return response.get('AbstractText') or response.get('Abstract') or f"Результатов по запросу '{query}' не найдено."
    except Exception as e:
        logger.error(f"❌ Ошибка поиска: {e}")
        return f"Ошибка поиска: {str(e)}"

@tool
def get_system_info() -> str:
    """Get basic system information."""
    import platform
    return f"Система: {platform.system()} {platform.release()}"

@tool
def calculate(expression: str) -> str:
    """Calculate mathematical expressions safely."""
    try:
        allowed_chars = set('0123456789+-*/.() ')
        if not all(c in allowed_chars for c in expression):
            return "Недопустимые символы."
        return f"Результат: {eval(expression)}"
    except Exception as e:
        logger.error(f"❌ Ошибка вычисления: {e}")
        return f"Ошибка вычисления: {str(e)}"

# --- ОСНОВНАЯ ФУНКЦИЯ ПРИЛОЖЕНИЯ ---
def create_app():
    app = FastAPI(title="Orchestrator Backend")

    @app.on_event("startup")
    async def on_startup():
        global ollama_process, sdk
        logger.info("🚀 Backend: Запуск...")
        try:
            ollama_process = subprocess.Popen([OLLAMA_PATH, "serve"], creationflags=subprocess.CREATE_NO_WINDOW)
            await asyncio.sleep(5)
            logger.info(f"✅ Backend: Процесс Ollama запущен.")
        except Exception as e:
            logger.error(f"Критическая ошибка при запуске Ollama: {e}", exc_info=True)
            os.kill(os.getpid(), signal.SIGTERM)
            return

        try:
            logger.info("🤖 Backend: Инициализация агентов...")
            llm = OllamaLLM(model=DEFAULT_MODEL, base_url="http://127.0.0.1:11434")
            tools = [search_the_web, get_system_info, calculate]
            prompt = PromptTemplate.from_template(
                """You are a helpful AI assistant called "The Orchestrator". You must respond in Russian language.
                Available tools: {tools}
                Use the following format:
                Question: the input question you must answer
                Thought: you should always think about what to do
                Action: the action to take, should be one of [{tool_names}]
                Action Input: the input to the action
                Observation: the result of the action
                Thought: I now know the final answer
                Final Answer: the final answer to the original input question in Russian
                Begin!
                Question: {input}
                Thought: {agent_scratchpad}"""
            )
            react_agent = create_react_agent(llm, tools, prompt)
            
            # --- ВОТ ГЛАВНОЕ ИСПРАВЛЕНИЕ: ЭТА СТРОКА ДЕЛАЕТ АГЕНТ УСТОЙЧИВЫМ ---
            agent_executor = AgentExecutor(
                agent=react_agent,
                tools=tools,
                verbose=True,
                handle_parsing_errors=True # <-- ЭТО СПАСАЕТ ОТ ПАДЕНИЙ
            )
            
            copilot_agent = LangGraphAgent(name="OrchestratorAgent", description="Помощник", agent=agent_executor)
            sdk = CopilotKitSDK(agents=[copilot_agent])
            logger.info("✅ Backend: Агенты и SDK успешно инициализированы.")
        except Exception as e:
            logger.error(f"❌ Не удалось инициализировать агентов: {e}", exc_info=True)
            raise

    app.add_middleware(CORSMiddleware, allow_origins=["*"], allow_credentials=True, allow_methods=["*"], allow_headers=["*"])

    @app.get("/health")
    async def health_check():
        if sdk is not None: return {"status": "healthy"}
        raise HTTPException(status_code=503, detail="Server is initializing...")

    # Используем официальный и рабочий способ регистрации эндпоинта
    def get_sdk():
        if sdk is None: raise HTTPException(status_code=503, detail="SDK не инициализирован")
        return sdk
    add_fastapi_endpoint(app, get_sdk, "/api/copilotkit")

    if os.path.exists("my-copilot-app/dist"):
        app.mount("/", StaticFiles(directory="my-copilot-app/dist", html=True), name="static")

    return app

if __name__ == "__main__":
    app = create_app()
    uvicorn.run(app, host="127.0.0.1", port=8000, log_level="info")