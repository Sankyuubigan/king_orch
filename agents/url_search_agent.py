# agents/url_search_agent.py - НОВЫЙ АГЕНТ ДЛЯ ПОИСКА URL

from .base_agent import BaseAgent
import re
from llama_cpp import Llama

class URLSearchAgent(BaseAgent):
    def __init__(self, llm_instance: Llama, tools_config: dict, log_callback):
        system_prompt = """Твоя единственная задача — найти URL для указанного сайта или страницы.
Ты должен использовать инструмент `web_search` для поиска, а затем извлечь из результатов ТОЛЬКО ОДИН, наиболее релевантный URL.

Пример:
Задача: Найди официальную страницу Павла Дурова в ВК.
Твой ход мыслей: "Я использую web_search с запросом 'официальная страница Павла Дурова вконтакте'".
Твой ответ:
[TOOL_CALL] {"tool": "web_search", "params": {"goal": "официальная страница Павла Дурова вконтакте"}}
"""
        super().__init__(llm_instance, system_prompt, tools_config, log_callback)

    def find_url(self, topic: str) -> str:
        self.history = []
        
        # Шаг 1: Поиск информации
        search_results, finished = self.execute_step(f"Найди URL для: '{topic}'")
        if finished:
            return f"Ошибка: не удалось использовать web_search для поиска '{topic}'"
            
        # Шаг 2: Извлечение URL из результатов
        extraction_task = f"""Из этого текста извлеки самый первый и самый релевантный URL.
Твой ответ должен содержать ТОЛЬКО URL и ничего больше.
Пример: https://example.com/page

Текст для анализа:
{search_results}
"""
        url, _ = self.execute_step(extraction_task)
        
        # Простая очистка, чтобы вернуть только URL
        url_match = re.search(r'https?://[^\s/$.?#].[^\s]*', url)
        if url_match:
            return url_match.group(0)
        
        return "Ошибка: не удалось извлечь URL из результатов поиска."