# agents/search_analyst_agent.py - ОБНОВЛЕН ПРОМПТ ДЛЯ НОВЫХ ИНСТРУМЕНТОВ

from .base_agent import BaseAgent
import json
from llama_cpp import Llama

class SearchAnalystAgent(BaseAgent):
    def __init__(self, llm_instance: Llama, tools_config: dict, log_callback):
        system_prompt = """Ты — ведущий аналитик-поисковик. Твоя задача — находить в интернете релевантные источники по заданной теме и извлекать из них URL-адреса или названия репозиториев.
Ты работаешь в два этапа:
1. Выбираешь НАИБОЛЕЕ ПОДХОДЯЩИЙ инструмент и используешь его для поиска.
2. Анализируешь результаты и формируешь из них JSON-список.

Доступные инструменты для поиска:
- `web_search`: для общего поиска информации в интернете.
- `github_search`: ИСПОЛЬЗУЙ ЭТОТ ИНСТРУМЕНТ, если запрос явно касается поиска репозиториев, кода или проектов на GitHub.
- `gitlab_search`: ИСПОЛЬЗУЙ ЭТОТ ИНСТРУМЕНТ, если запрос явно касается поиска репозиториев, кода или проектов на GitLab.

Твой финальный ответ после анализа результатов должен быть ТОЛЬКО списком в формате JSON.
Пример:
["https://www.url1.com", "user/repo1", "https://news.url3.org/article", "gitlab_user/project_name"]
Не пиши ничего, кроме JSON-списка."""
        super().__init__(llm_instance, system_prompt, tools_config, log_callback)

    def analyze_and_select_sources(self, topic: str) -> list[str]:
        self.history = []
        
        # --- ШАГ 1: ВЫБОР ИНСТРУМЕНТА И ПОИСК ---
        task_1 = f"Выбери лучший инструмент и используй его для поиска по теме '{topic}'."
        self.log(f"[{self.__class__.__name__}] Шаг 1: Выбираю инструмент и выполняю поиск по теме '{topic}'...")
        search_results, finished = self.execute_step(task_1)
        
        if finished:
            self.log(f"[{self.__class__.__name__}] [ERROR] Агент не смог использовать инструмент поиска.")
            return []

        # --- ШАГ 2: АНАЛИЗ И ИЗВЛЕЧЕНИЕ РЕЗУЛЬТАТОВ ---
        task_2 = f"""Из полученного текста с результатами поиска извлеки 3-5 самых релевантных URL или названий репозиториев.
Твой финальный ответ должен быть ТОЛЬКО списком в формате JSON.
Не пиши ничего, кроме JSON-списка."""
        
        self.log(f"[{self.__class__.__name__}] Шаг 2: Анализирую результаты и извлекаю ссылки/репозитории...")
        response, _ = self.execute_step(task_2)
        
        try:
            json_part_start = response.find('[')
            json_part_end = response.rfind(']') + 1
            if json_part_start == -1 or json_part_end == 0:
                raise ValueError("JSON-массив не найден в ответе модели.")

            json_part = response[json_part_start:json_part_end]
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