import logging
from typing import TypedDict
from langchain_community.chat_models import ChatLlamaCpp
from mcp_use import MCPClient, MCPAgent
from langgraph.graph import StateGraph, END

from prompts.research_prompts import RESEARCH_SYSTEM_PROMPT

logger = logging.getLogger(__name__)

# --- 1. Определение состояния графа ---
class GraphState(TypedDict):
    task: str
    result: str

# --- 2. Определение узла графа ---
def run_agent_node(state: GraphState, llm: ChatLlamaCpp, mcp_client: MCPClient):
    """
    Единственный узел, который создает и запускает специализированного агента.
    """
    logger.info("--- RESEARCH_GRAPH: Запуск агента для исследований ---")
    
    agent = MCPAgent(
        llm=llm,
        client=mcp_client,
        system_prompt=RESEARCH_SYSTEM_PROMPT,
        use_server_manager=True,
        max_steps=20
    )
    
    final_result = agent.run(state["task"])
    logger.info(f"--- RESEARCH_GRAPH: Агент завершил работу с результатом: {final_result} ---")
    
    return {"result": final_result}

# --- 3. Сборка графа ---
def create_research_graph(llm: ChatLlamaCpp, mcp_client: MCPClient):
    workflow = StateGraph(GraphState)
    
    workflow.add_node("agent_executor", lambda state: run_agent_node(state, llm, mcp_client))

    workflow.set_entry_point("agent_executor")
    workflow.add_edge("agent_executor", END)

    app = workflow.compile()
    logger.info("Граф для исследований (архитектура 'единый агент') успешно скомпилирован.")
    return app