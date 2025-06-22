# agents/code_retriever_agent.py - НОВЫЙ АГЕНТ-"БИБЛИОТЕКАРЬ"

from .base_agent import BaseAgent
from llama_cpp import Llama
import json

class CodeRetrieverAgent(BaseAgent):
    def __init__(self, llm_instance: Llama, tools_config: dict, log_callback):
        system_prompt = """Ты — AI-ассистент, эксперт по семантическому поиску кода. Твоя единственная задача — взять текстовый запрос на естественном языке и, используя инструмент `chroma_query`, найти в векторной базе данных наиболее релевантные фрагменты кода.

**ПРОТОКОЛ РАБОТЫ:**
1.  Получи задачу, например: "Найди примеры работы с API".
2.  Сформируй вызов инструмента `chroma_query`.
    - `collection_name`: Всегда используй `"project_code_memory"`.
    - `query_texts`: Передай сюда исходный запрос пользователя в виде списка из одной строки.
    - `n_results`: Запрашивай разумное количество результатов, обычно 3 или 5.
3.  Твой ответ ДОЛЖЕН быть ТОЛЬКО вызовом инструмента.

**ПРИМЕР:**
Задача: "Как в этом проекте подключаются к базе данных?"
Твой ответ:
[TOOL_CALL] {"tool": "chroma_query", "params": {"collection_name": "project_code_memory", "query_texts": ["Как в этом проекте подключаются к базе данных?"], "n_results": 3}}
"""
        super().__init__(llm_instance, system_prompt, tools_config, log_callback)

    def retrieve_code_snippets(self, query: str, n_results: int = 3) -> dict:
        """
        Выполняет семантический поиск по базе кода.
        """
        self.history = []
        task = f"Найди {n_results} наиболее релевантных фрагментов кода для запроса: '{query}'"
        
        # `execute_step` вернет результат вызова инструмента, который уже является dict
        search_results, _ = self.execute_step(task)
        
        return search_results