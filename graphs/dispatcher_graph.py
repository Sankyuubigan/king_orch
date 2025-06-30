import logging
from typing import TypedDict, Dict, Any
import asyncio
import time
from langchain_community.chat_models import ChatLlamaCpp
from mcp_use import MCPClient, MCPAgent
from langgraph.graph import StateGraph, END

from prompts.dispatcher_prompts import dispatcher_prompt_template
# ИЗМЕНЕНИЕ: Импортируем новую утилиту
from utils.text_utils import extract_thoughts

logger = logging.getLogger(__name__)

class DispatcherState(TypedDict):
    task: str
    route: str
    # ИЗМЕНЕНИЕ: result теперь может быть словарем
    result: Dict[str, Any] | str

def router_node(state: DispatcherState, llm: ChatLlamaCpp):
    """
    Определяет, какой граф должен обработать задачу. Возвращает ТОЛЬКО ключевое слово.
    """
    logger.info("--- ВХОД В УЗЕЛ: МАРШРУТИЗАТОР (БЕЗОПАСНЫЙ РЕЖИМ) ---")
    
    lightweight_client = MCPClient()
    dispatcher_agent = MCPAgent(
        llm=llm,
        client=lightweight_client,
        system_prompt=dispatcher_prompt_template,
        use_server_manager=False,
        max_steps=1
    )
    
    start_time = time.perf_counter()
    raw_route_output = asyncio.run(dispatcher_agent.run(state["task"])).strip().lower()
    end_time = time.perf_counter()
    logger.info(f"[TIMER] Routing decision took: {end_time - start_time:.2f}s")
    
    route_keywords = ["coding", "research", "browser", "general_conversation"]
    final_route = "general_conversation"

    if raw_route_output in route_keywords:
        final_route = raw_route_output
    else:
        logger.warning(f"Ответ маршрутизатора не прошел строгую проверку. Используется маршрут по умолчанию. (Сырой ответ: '{raw_route_output}')")

    logger.info(f"Принято решение о маршрутизации задачи в граф: '{final_route}'")
    return {"route": final_route}

def llm_node(state: DispatcherState, llm: ChatLlamaCpp):
    """
    Вызывает LLM и разделяет ее ответ на "чистый ответ" и "мысли".
    """
    logger.info("--- ВХОД В УЗЕЛ: ОБЩЕНИЕ (ПРЯМОЙ ВЫЗОВ) ---")
    
    prompt = f"Ты — полезный AI-ассистент. Ответь на вопрос пользователя кратко и по делу.\n\nВопрос: {state['task']}"
    
    start_time = time.perf_counter()
    response = llm.invoke(prompt)
    end_time = time.perf_counter()
    logger.info(f"[TIMER] Final response generation took: {end_time - start_time:.2f}s")

    # ИЗМЕНЕНИЕ: Используем экстрактор для разделения ответа
    structured_result = extract_thoughts(response.content)
    
    logger.info("Ответ на общий вопрос сгенерирован и разделен на ответ/мысли.")
    # Возвращаем структурированный словарь
    return {"result": structured_result}

def route_logic(state: DispatcherState):
    """
    Возвращает имя следующего узла на основе решения маршрутизатора.
    """
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

def create_dispatcher_graph(llm: ChatLlamaCpp, coding_graph, research_graph, browser_graph):
    """
    Создает и компилирует главный граф-диспетчер с безопасной архитектурой.
    """
    workflow = StateGraph(DispatcherState)

    workflow.add_node("router", lambda state: router_node(state, llm))
    workflow.add_node("general_conversation", lambda state: llm_node(state, llm))
    
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
    logger.info("Главный граф-диспетчер (архитектура 'Безопасный маршрутизатор') успешно скомпилирован.")
    return app