# agents/search_analyst_agent.py - ОБНОВЛЕН ДЛЯ РАБОТЫ С JSON-ВЫВОДОМ

from .base_agent import BaseAgent
import json
import re
from llama_cpp import Llama

class SearchAnalystAgent(BaseAgent):
    def __init__(self, llm_instance: Llama, tools_config: dict, log_callback):
        system_prompt = """Ты — ведущий аналитик-поисковик. Твоя задача — взять запрос, использовать инструмент `web_search` и проанализировать полученный JSON-ответ.

**ПРОЦЕСС РАБОТЫ:**
1.  Ты получаешь задачу, например: "Найди официальный сайт OpenAI".
2.  Ты используешь инструмент `web_search` с наиболее точным запросом.
    `[TOOL_CALL] {"tool": "web_search", "params": {"query": "official website of OpenAI"}}`
3.  Инструмент возвращает JSON-объект со списком результатов в поле `results`. Каждый результат — это словарь с ключами `url`, `title` и `content`.
4.  Твоя задача — изучить эти результаты и извлечь из них 3-5 самых релевантных URL.
5.  Твой финальный ответ должен быть ТОЛЬКО списком URL в формате JSON. Не пиши ничего, кроме самого JSON-массива.

Пример финального ответа:
["https://openai.com/", "https://openai.com/blog", "https://platform.openai.com/"]
"""
        super().__init__(llm_instance, system_prompt, tools_config, log_callback)

    def analyze_and_select_sources(self, topic: str) -> list[str]:
        self.history = []
        
        task_1 = f"Используй инструмент web_search, чтобы найти информацию по теме: '{topic}'."
        self.log(f"[{self.__class__.__name__}] Шаг 1: Выполняю поиск по теме '{topic}'...")
        # search_results - это будет dict или list из BaseAgent
        search_results, finished = self.execute_step(task_1)
        
        if finished or not isinstance(search_results, dict) or "results" not in search_results:
            self.log(f"[{self.__class__.__name__}] [ERROR] Агент не смог выполнить поиск или получил некорректный результат. Ответ был: {search_results}")
            return []

        task_2 = f"""Вот JSON с результатами поиска. Извлеки из него 3-5 самых релевантных URL и верни их в виде JSON-списка.

Результаты поиска:
{json.dumps(search_results, ensure_ascii=False, indent=2)}

Твой финальный ответ должен быть ТОЛЬКО списком в формате JSON.
"""
        
        self.log(f"[{self.__class__.__name__}] Шаг 2: Анализирую результаты и извлекаю URL...")
        response, _ = self.execute_step(task_2)
        
        try:
            # Используем регулярное выражение для надежного извлечения JSON-массива
            match = re.search(r'(\[[\s\S]*?\])', str(response))
            if not match:
                raise ValueError("JSON-массив не найден в ответе модели.")

            json_part = match.group(1)
            urls = json.loads(json_part)
            
            if isinstance(urls, list) and all(isinstance(u, str) for u in urls):
                self.log(f"[{self.__class__.__name__}] Выбрал {len(urls)} источников.")
                return urls
            else:
                self.log(f"[{self.__class__.__name__}] [ERROR] Ответ модели не является списком строк.")
                return []
        except Exception as e:
            self.log(f"[{self.__class__.__name__}] [ERROR] Не смог извлечь JSON из ответа: {e}\nОтвет был: {response}")
            return []