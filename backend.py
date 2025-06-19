# backend.py - УПРОЩЕННАЯ И НАДЕЖНАЯ ВЕРСИЯ

import uvicorn
from fastapi import FastAPI, HTTPException
from fastapi.staticfiles import StaticFiles
from fastapi.middleware.cors import CORSMiddleware
import os
import logging

from copilotkit import CopilotKitSDK, LangGraphAgent
from copilotkit.integrations.fastapi import add_fastapi_endpoint
from langchain_ollama import OllamaLLM
from langchain_core.tools import tool
from langchain.prompts import PromptTemplate
from langchain.agents import AgentExecutor, create_react_agent

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)

# --- ОПРЕДЕЛЕНИЕ ИНСТРУМЕНТОВ ---
@tool
def search_the_web(query: str): /* ... ваш код ... */
@tool
def get_system_info(): /* ... ваш код ... */
@tool
def calculate(expression: str): /* ... ваш код ... */

# --- ИНИЦИАЛИЗАЦИЯ (выполняется один раз при импорте) ---
logger.info("🤖 Инициализация агентов и SDK...")

try:
    llm = OllamaLLM(model="llama3:8b", base_url="http://127.0.0.1:11434")
    tools = [search_the_web, get_system_info, calculate]

    prompt = PromptTemplate.from_template(
        """You are a helpful AI assistant called "The Orchestrator"...(ваш полный промпт)..."""
    )

    react_agent = create_react_agent(llm, tools, prompt)
    agent_executor = AgentExecutor(
        agent=react_agent,
        tools=tools,
        verbose=True,
        handle_parsing_errors=True
    )
    
    copilot_agent = LangGraphAgent(name="OrchestratorAgent", description="Помощник", agent=agent_executor)
    sdk = CopilotKitSDK(agents=[copilot_agent])

    logger.info("✅ Агенты и SDK успешно инициализированы.")
except Exception as e:
    logger.error(f"❌ КРИТИЧЕСКАЯ ОШИБКА при инициализации SDK: {e}", exc_info=True)
    # Если мы здесь, приложение не сможет работать, но мы все равно создадим app, чтобы не было ошибки импорта.
    sdk = None

# --- СОЗДАНИЕ ПРИЛОЖЕНИЯ FASTAPI ---
app = FastAPI(title="Orchestrator Backend")

app.add_middleware(CORSMiddleware, allow_origins=["*"], allow_credentials=True, allow_methods=["*"], allow_headers=["*"])

if sdk:
    add_fastapi_endpoint(app, sdk, "/api/copilotkit")
    logger.info("✅ Эндпоинт CopilotKit успешно добавлен.")
else:
    logger.error("❌ SDK не был инициализирован, эндпоинт CopilotKit неактивен.")

@app.get("/health")
async def health_check():
    """Простая проверка. Если сервер отвечает, значит, он жив."""
    return {"status": "backend_is_running"}

if os.path.exists("my-copilot-app/dist"):
    app.mount("/", StaticFiles(directory="my-copilot-app/dist", html=True), name="static")

# Этот файл больше не запускается напрямую, но блок оставлен для возможных тестов
if __name__ == "__main__":
    uvicorn.run(app, host="127.0.0.1", port=8000, log_level="info")