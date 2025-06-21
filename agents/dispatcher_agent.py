# agents/dispatcher_agent.py - НОВЫЙ АГЕНТ-МАРШРУТИЗАТОР

from .base_agent import BaseAgent
from llama_cpp import Llama

class DispatcherAgent(BaseAgent):
    def __init__(self, llm_instance: Llama, log_callback):
        system_prompt = """Твоя единственная задача — классифицировать запрос пользователя и определить, какая команда агентов должна его обработать.
Ты должен ответить ОДНИМ СЛОВОМ из следующего списка:
- `coding`: если запрос связан с написанием или изменением кода, созданием файлов, скриптов, программ, выполнением команд в терминале.
- `research`: если запрос связан с поиском информации, анализом веб-страниц, поиском репозиториев, составлением отчетов или ответом на общие вопросы.
- `general_conversation`: если это простое приветствие, прощание или короткий диалог, не требующий выполнения сложных задач.

Примеры:
Запрос: "привет, как дела?" -> Ответ: general_conversation
Запрос: "напиши скрипт на питоне для парсинга сайта" -> Ответ: coding
Запрос: "найди мне информацию о лучших фреймворках для python" -> Ответ: research
Запрос: "создай файл main.py и запусти его" -> Ответ: coding
Запрос: "какие есть mcp-серверы для работы с github?" -> Ответ: research

Твой ответ должен содержать ТОЛЬКО одно из этих трех слов. Никаких объяснений.
"""
        # У этого агента нет инструментов, поэтому tools_config - пустой словарь
        super().__init__(llm_instance, system_prompt, {}, log_callback)

    def choose_crew(self, goal: str) -> str:
        self.history = []
        # Мы ожидаем однословный ответ, поэтому max_tokens можно сильно ограничить
        self.llm.max_tokens = 10
        category, _ = self.execute_step(f"Классифицируй этот запрос: \"{goal}\"")
        self.llm.max_tokens = 1024 # Возвращаем обратно для других агентов
        
        # Очистка ответа модели до одного из трех ключевых слов
        clean_category = category.strip().lower().replace(".", "")
        if "coding" in clean_category:
            return "coding"
        if "research" in clean_category:
            return "research"
        return "general_conversation"