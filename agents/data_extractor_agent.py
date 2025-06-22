# agents/data_extractor_agent.py - ВОССТАНОВЛЕН ДЛЯ РАБОТЫ ЧЕРЕЗ СЕТЬ

from .base_agent import BaseAgent
from llama_cpp import Llama

class DataExtractorAgent(BaseAgent):
    def __init__(self, llm_instance: Llama, tools_config: dict, log_callback):
        system_prompt = """Ты — специалист по извлечению данных. Твоя единственная задача — использовать инструмент `ashra_extract` для получения структурированной информации со страницы.
Инструмент принимает два параметра: `url` и `prompt` (что именно нужно извлечь, на английском).

Пример:
Задача: Извлеки текст последнего поста со страницы 'https://vk.com/durov'.
Твой ответ:
[TOOL_CALL] {"tool": "ashra_extract", "params": {"url": "https://vk.com/durov", "prompt": "the text of the latest post on the wall"}}
"""
        super().__init__(llm_instance, system_prompt, tools_config, log_callback)

    def extract_data(self, url: str, prompt: str) -> str:
        self.history = []
        # Вызываем инструмент напрямую через _execute_tool
        result = self._execute_tool("ashra_extract", {"url": url, "prompt": prompt})
        return result