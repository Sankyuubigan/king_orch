# agents/docs_expert_agent.py - НОВЫЙ АГЕНТ ДЛЯ РАБОТЫ С RAG

from .base_agent import BaseAgent
from llama_cpp import Llama

class DocsExpertAgent(BaseAgent):
    def __init__(self, llm_instance: Llama, tools_config: dict, log_callback):
        system_prompt = """Ты — эксперт по технической документации. Твоя единственная задача — использовать инструмент `ask_docs` для ответа на вопрос пользователя.
Ты должен передать вопрос пользователя в параметр `query` инструмента.

Пример:
Задача: Расскажи о FastAPI.
Твой ответ:
[TOOL_CALL] {"tool": "ask_docs", "params": {"query": "Что такое FastAPI?"}}
"""
        super().__init__(llm_instance, system_prompt, tools_config, log_callback)

    def answer_from_docs(self, question: str) -> str:
        self.history = []
        result = self._execute_tool("ask_docs", {"query": question})
        return result