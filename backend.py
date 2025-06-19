# backend.py

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
from stagehand import Stagehand

# --- НАСТРОЙКИ ---
OLLAMA_PATH = r"D:\Projects\universal_orchestrator\ollama_runtime\ollama.exe"
DEFAULT_MODEL = "llama3:8b"

# --- ГЛОБАЛЬНЫЕ ПЕРЕМЕННЫЕ ---
ollama_process = None

# Настройка логирования
logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)

def create_app():
    app = FastAPI(title="Orchestrator Backend")
    
    # Глобальная переменная для SDK
    sdk = None
    agent_executor = None

    @app.on_event("startup")
    async def on_startup():
        nonlocal sdk, agent_executor
        global ollama_process
        
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
            
            # Инициализируем Stagehand с обработкой ошибок
            try:
                stagehand_agent = Stagehand(
                    env="LOCAL", 
                    model=f"ollama/{DEFAULT_MODEL}", 
                    browser_options={"headless": True}
                )
                stagehand_available = True
            except Exception as e:
                logger.warning(f"⚠️ Stagehand initialization failed: {e}")
                stagehand_agent = None
                stagehand_available = False

            @tool
            def search_the_web(query: str) -> str:
                """Searches the web for the given query using Stagehand."""
                if not stagehand_available:
                    return f"Web search unavailable. Query was: {query}"
                
                logger.info(f"🔎 TOOL: Executing Stagehand with query: {query}")
                try:
                    result = stagehand_agent.invoke(
                        goal=query, 
                        url="https://www.google.com"
                    )
                    return result.get('answer', 'Failed to get an answer.')
                except Exception as e:
                    logger.error(f"❌ Stagehand error: {e}")
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
Final Answer: the final answer to the original input question

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
                max_iterations=3
            )
            
            # Создаем LangGraphAgent
            copilot_agent = LangGraphAgent(
                agent=agent_executor,
                name="OrchestratorAgent",
                description="A helpful assistant that can search the web, get system info, calculate, and answer questions."
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

    # --- COPILOTKIT INTEGRATION ---
    @app.middleware("http")
    async def ensure_sdk(request: Request, call_next):
        # Ждем инициализации SDK для CopilotKit эндпоинтов
        if request.url.path.startswith("/copilotkit") or request.url.path == "/chat":
            max_wait = 30  # секунды
            waited = 0
            while sdk is None and waited < max_wait:
                await asyncio.sleep(0.5)
                waited += 0.5
            
            if sdk is None:
                return JSONResponse(
                    status_code=503,
                    content={"error": "SDK not initialized yet, please try again"}
                )
        
        response = await call_next(request)
        return response

    # --- ОСНОВНЫЕ ЭНДПОИНТЫ ---
    
    @app.post("/chat")
    async def chat_endpoint(request: Request):
        """Обработка chat запросов с правильным выполнением агента."""
        try:
            body = await request.json()
            logger.info(f"📨 Chat request received")
            
            # Извлекаем сообщения из GraphQL запроса
            variables = body.get('variables', {})
            data = variables.get('data', {})
            messages = data.get('messages', [])
            
            if not messages:
                return JSONResponse({
                    "error": "No messages found in request",
                    "status": "error"
                })
            
            # Получаем последнее пользовательское сообщение
            user_messages = [msg for msg in messages 
                           if msg.get('textMessage', {}).get('role') == 'user']
            
            if not user_messages:
                return JSONResponse({
                    "error": "No user messages found",
                    "status": "error"  
                })
            
            last_message = user_messages[-1]
            user_input = last_message.get('textMessage', {}).get('content', '')
            
            logger.info(f"💬 Processing user message: {user_input}")
            
            # Выполняем агента напрямую через agent_executor
            response_text = "Извините, агент еще не готов к работе."
            
            if agent_executor:
                try:
                    logger.info("🔄 Executing agent with user input...")
                    
                    # Выполняем агента в отдельном потоке
                    result = await asyncio.to_thread(
                        agent_executor.invoke,
                        {"input": user_input}
                    )
                    
                    # Извлекаем ответ
                    if isinstance(result, dict):
                        response_text = result.get('output', str(result))
                    else:
                        response_text = str(result)
                    
                    logger.info(f"✅ Agent response: {response_text[:100]}...")
                    
                except Exception as agent_error:
                    logger.error(f"❌ Agent execution error: {agent_error}")
                    response_text = f"Произошла ошибка при выполнении запроса: {str(agent_error)}"
            
            # Возвращаем в правильном формате
            return JSONResponse({
                "data": {
                    "threadId": data.get('threadId', f"thread_{int(time.time())}"),
                    "runId": f"run_{int(time.time())}",
                    "messages": [{
                        "id": f"msg_{int(time.time())}",
                        "textMessage": {
                            "content": response_text,
                            "role": "assistant"
                        },
                        "createdAt": int(time.time() * 1000),  # timestamp в миллисекундах
                        "__typename": "Message"
                    }],
                    "status": {
                        "code": "SUCCESS",
                        "__typename": "BaseResponseStatus"
                    }
                }
            })
            
        except Exception as e:
            logger.error(f"❌ Chat endpoint error: {e}")
            return JSONResponse(
                status_code=500,
                content={
                    "error": str(e),
                    "status": "error"
                }
            )

    @app.get("/health")
    async def health_check():
        """Проверка здоровья сервиса."""
        return {
            "status": "healthy",
            "ollama_running": ollama_process and ollama_process.poll() is None,
            "sdk_initialized": sdk is not None,
            "agent_executor_ready": agent_executor is not None,
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
            "sdk_methods": [method for method in dir(sdk) if not method.startswith('_')],
            "agent_executor_available": agent_executor is not None
        }

    # Добавляем официальный CopilotKit эндпоинт
    try:
        add_fastapi_endpoint(app, lambda: sdk, "/copilotkit")
        logger.info("✅ Backend: Official CopilotKit FastAPI endpoint added successfully.")
    except Exception as e:
        logger.warning(f"⚠️ Could not add official CopilotKit endpoint: {e}")

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