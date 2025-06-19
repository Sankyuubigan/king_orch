# backend.py - ИСПРАВЛЕННАЯ ВЕРСИЯ

import uvicorn
import subprocess
import os
import signal
import time
import asyncio
from fastapi import FastAPI, Request, HTTPException
from fastapi.staticfiles import StaticFiles
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import StreamingResponse, JSONResponse
import json
import logging
from typing import Dict, Any, List

# --- ПРАВИЛЬНЫЕ ИМПОРТЫ ---
from copilotkit import CopilotKitSDK, LangGraphAgent
from copilotkit.integrations.fastapi import add_fastapi_endpoint
from langchain_community.llms import Ollama
from langchain.agents import AgentExecutor, create_react_agent
from langchain_core.tools import tool
from langchain.prompts import PromptTemplate
from langchain_core.messages import HumanMessage, AIMessage

# --- НАСТРОЙКИ ---
OLLAMA_PATH = r"D:\Projects\universal_orchestrator\ollama_runtime\ollama.exe"
DEFAULT_MODEL = "llama3:8b"

# --- ГЛОБАЛЬНЫЕ ПЕРЕМЕННЫЕ ---
ollama_process = None
sdk = None  # Глобальная переменная для SDK

# Настройка логирования
logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)

def create_app():
    app = FastAPI(title="Orchestrator Backend")
    
    @app.on_event("startup")
    async def on_startup():
        global ollama_process, sdk
        
        logger.info("🚀 Backend: FastAPI server starting up...")
        logger.info(f"🔥 Backend: Starting Ollama from: {OLLAMA_PATH}")
        
        try:
            ollama_process = subprocess.Popen(
                [OLLAMA_PATH, "serve"], 
                creationflags=subprocess.CREATE_NO_WINDOW
            )
            logger.info(f"✅ Backend: Ollama process started with PID: {ollama_process.pid}")
            
            # Ждем запуска Ollama
            await asyncio.sleep(5)
            
        except FileNotFoundError:
            logger.error(f"❌ FATAL: Ollama executable not found at '{OLLAMA_PATH}'")
            os.kill(os.getpid(), signal.SIGTERM)
            return

        # Инициализация агентов после запуска Ollama
        try:
            logger.info("🤖 Backend: Initializing agents...")
            
            llm = Ollama(model=DEFAULT_MODEL, base_url="http://127.0.0.1:11434")
            
            @tool
            def search_the_web(query: str) -> str:
                """Searches the web for the given query."""
                logger.info(f"🔎 TOOL: Web search with query: {query}")
                try:
                    import requests
                    # Простой поиск через DuckDuckGo API
                    url = f"https://api.duckduckgo.com/?q={query}&format=json&no_html=1"
                    response = requests.get(url, timeout=10)
                    data = response.json()
                    
                    if data.get('AbstractText'):
                        return f"Search result: {data['AbstractText']}"
                    elif data.get('Abstract'):
                        return f"Search result: {data['Abstract']}"
                    else:
                        return f"Search completed for: {query}, but no specific results found."
                        
                except Exception as e:
                    logger.error(f"❌ Web search error: {e}")
                    return f"Search failed: {str(e)}"

            @tool 
            def get_system_info() -> str:
                """Get basic system information."""
                import platform
                return f"System: {platform.system()} {platform.release()}, Python: {platform.python_version()}"

            @tool
            def calculate(expression: str) -> str:
                """Calculate mathematical expressions safely."""
                try:
                    # Простая и безопасная математика
                    allowed_chars = set('0123456789+-*/.() ')
                    if not all(c in allowed_chars for c in expression):
                        return "Invalid characters in mathematical expression"
                    
                    result = eval(expression)
                    return f"Result: {result}"
                except Exception as e:
                    return f"Calculation error: {str(e)}"

            tools = [search_the_web, get_system_info, calculate]
            
            # Создаем собственный промпт для React агента
            prompt = PromptTemplate.from_template(
                """You are a helpful AI assistant called "The Orchestrator". You have access to several tools that can help you answer questions and perform tasks.

You must respond in Russian language, as the user is Russian-speaking.

Available tools:
{tools}

Use the following format:

Question: the input question you must answer
Thought: you should always think about what to do
Action: the action to take, should be one of [{tool_names}]
Action Input: the input to the action
Observation: the result of the action
... (this Thought/Action/Action Input/Observation can repeat N times)
Thought: I now know the final answer
Final Answer: the final answer to the original input question in Russian

Begin!

Question: {input}
Thought: {agent_scratchpad}"""
            )
            
            react_agent = create_react_agent(llm, tools, prompt)
            agent_executor = AgentExecutor(
                agent=react_agent, 
                tools=tools, 
                verbose=True,
                handle_parsing_errors=True,
                max_iterations=5,
                return_intermediate_steps=True
            )
            
            # Создаем LangGraphAgent
            copilot_agent = LangGraphAgent(
                name="OrchestratorAgent",
                description="A helpful assistant that can search the web, get system info, calculate, and answer questions in Russian.",
                agent=agent_executor
            )
            
            # Создаем SDK
            sdk = CopilotKitSDK(agents=[copilot_agent])
            logger.info("✅ Backend: Agents and SDK initialized successfully.")
            
        except Exception as e:
            logger.error(f"❌ Failed to initialize agents: {e}")
            raise

    @app.on_event("shutdown")
    def on_shutdown():
        global ollama_process
        logger.info("💀 Backend: Shutting down...")
        if ollama_process and ollama_process.poll() is None:
            logger.info(f"🛑 Backend: Terminating Ollama process (PID: {ollama_process.pid})")
            ollama_process.terminate()
            ollama_process.wait()
        logger.info("✅ Backend: Shutdown complete.")

    # --- MIDDLEWARE ---
    app.add_middleware(
        CORSMiddleware,
        allow_origins=["*"],
        allow_credentials=True,
        allow_methods=["*"],
        allow_headers=["*"]
    )

    # --- ОСНОВНЫЕ ЭНДПОИНТЫ ---
    
    @app.get("/health")
    async def health_check():
        """Проверка здоровья сервиса."""
        return {
            "status": "healthy",
            "ollama_running": ollama_process and ollama_process.poll() is None,
            "sdk_initialized": sdk is not None,
            "agents_count": len(sdk.agents) if sdk else 0
        }

    @app.get("/info")
    async def info():
        """Информация о доступных агентах и методах SDK."""
        if sdk is None:
            return {
                "status": "initializing",
                "message": "SDK is still initializing"
            }
        
        return {
            "status": "running",
            "agents": [
                {
                    "name": agent.name,
                    "description": agent.description
                } for agent in sdk.agents
            ],
            "sdk_methods": [method for method in dir(sdk) if not method.startswith('_')]
        }

    # ИСПРАВЛЕННАЯ функция для получения SDK
    def get_sdk():
        """Функция-коллбэк для получения SDK."""
        if sdk is None:
            logger.error("❌ SDK is not initialized yet!")
            raise HTTPException(status_code=503, detail="SDK not initialized")
        return sdk

    # Добавляем официальный CopilotKit эндпоинт с правильной настройкой
    try:
        add_fastapi_endpoint(app, get_sdk, "/copilotkit")
        logger.info("✅ Backend: Official CopilotKit FastAPI endpoint added successfully.")
    except Exception as e:
        logger.error(f"❌ Could not add official CopilotKit endpoint: {e}")

    # Дополнительная отладочная информация для CopilotKit
    @app.get("/copilotkit/debug")
    async def copilotkit_debug():
        """Отладочная информация для CopilotKit."""
        return {
            "sdk_initialized": sdk is not None,
            "agents": [agent.name for agent in sdk.agents] if sdk else [],
            "available_routes": [{"path": route.path, "methods": list(route.methods)} for route in app.routes if hasattr(route, 'path')],
            "middleware": [str(middleware) for middleware in app.user_middleware]
        }

    # Статические файлы в конце
    try:
        if os.path.exists("my-copilot-app/dist"):
            app.mount("/", StaticFiles(directory="my-copilot-app/dist", html=True), name="static")
            logger.info("✅ Static files mounted successfully")
        else:
            logger.warning("⚠️ Static files directory not found: my-copilot-app/dist")
    except Exception as e:
        logger.error(f"❌ Failed to mount static files: {e}")

    return app

if __name__ == "__main__":
    app = create_app()
    uvicorn.run(app, host="127.0.0.1", port=8000, log_level="info")