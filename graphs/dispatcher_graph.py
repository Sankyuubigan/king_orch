import logging
from typing import TypedDict
import asyncio
from langchain_community.chat_models import ChatLlamaCpp
from mcp_use import MCPClient, MCPAgent
from langgraph.graph import StateGraph, END

from prompts.dispatcher_prompts import dispatcher_prompt_template

logger = logging.getLogger(__name__)

# --- 1. Определение состояния графа ---
class DispatcherState(TypedDict):
    task: str
    route: str
    result: str

# --- 2. Определение узлов графа ---
def router_node(state: DispatcherState, llm: ChatLlamaCpp):
    """
    Определяет, какой граф должен обработать задачу, используя безопасный вызов через MCPAgent.
    """
    logger.info("--- ВХОД В УЗЕЛ: МАРШРУТИЗАТОР ---")
    
    # Создаем новый, ПУСТОЙ клиент. Он не знает о серверах из mcp_config.json.
    lightweight_client = MCPClient()
    
    # Создаем агента с этим пустым клиентом. Агент не будет пытаться инициализировать инструменты.
    # Он получит уже исправленный экземпляр llm.
    dispatcher_agent = MCPAgent(
        llm=llm,
        client=lightweight_client,
        system_prompt=dispatcher_prompt_template,
        use_server_manager=False,
        max_steps=1
    )
    
    # Корректно запускаем асинхронный метод agent.run()
    route = asyncio.run(dispatcher_agent.run(state["task"])).strip().lower()
    
    logger.info(f"Принято решение о маршрутизации задачи в граф: '{route}'")
    return {"route": route}

def general_node(state: DispatcherState, llm: ChatLlamaCpp):
    """
    Обрабатывает общие вопросы, не требующие инструментов.
    """
    logger.info("--- ВХОД В УЗЕЛ: ОБЩЕНИЕ ---")
    
    # Применяем тот же безопасный подход и здесь
    lightweight_client = MCPClient()
    general_agent = MCPAgent(
        llm=llm,
        client=lightweight_client,
        system_prompt="Ты — полезный AI-ассистент. Ответь на вопрос пользователя кратко и по делу.",
        use_server_manager=False,
        max_steps=1
    )
    
    response = asyncio.run(general_agent.run(state["task"]))
    logger.info("Ответ на общий вопрос сгенерирован.")
    return {"result": response}

# --- 3. Определение условных ребер ---
def route_logic(state: DispatcherState):
    """Возвращает имя следующего узла на основе решения маршрутизатора."""
    logger.info(f"--- ВЫБОР МАРШРУТА: {state['route']} ---")
    route = state['route']
    if "coding" in route:
        return "coding_graph"
    elif "research" in route:
        return "research_graph"
    elif "browser" in route:
        return "browser_graph"
    else:
        return "general_conversation"

# --- 4. Сборка графа ---
def create_dispatcher_graph(llm: ChatLlamaCpp, coding_graph, research_graph, browser_graph):
    """Создает и компилирует главный граф-диспетчер."""
    workflow = StateGraph(DispatcherState)

    workflow.add_node("router", lambda state: router_node(state, llm))
    workflow.add_node("general_conversation", lambda state: general_node(state, llm))
    
    workflow.add_node("coding_graph", coding_graph)
    workflow.add_node("research_graph", research_graph)
    workflow.add_node("browser_graph", browser_graph)

    workflow.set_entry_point("router")
    workflow.add_conditional_edges(
        "router",
        route_logic,
        {
            "coding_graph": "coding_graph",
            "research_graph": "research_graph",
            "browser_graph": "browser_graph",
            "general_conversation": "general_conversation"
        }
    )

    workflow.add_edge("coding_graph", END)
    workflow.add_edge("research_graph", END)
    workflow.add_edge("browser_graph", END)
    workflow.add_edge("general_conversation", END)

    app = workflow.compile()
    logger.info("Главный граф-диспетчер (архитектура 'исправленный LLM') успешно скомпилирован.")
    return app