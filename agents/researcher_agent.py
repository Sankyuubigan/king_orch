# agents/researcher_agent.py - Принимает llm_instance

from .base_agent import BaseAgent
from llama_cpp import Llama

class ResearcherAgent(BaseAgent):
    def __init__(self, llm_instance: Llama, tools_config: dict, log_callback):
        system_prompt = """Ты — младший научный сотрудник. Твоя задача — взять ОДИН URL, использовать инструмент `content_fetcher` чтобы прочитать его содержимое, и написать краткое, но емкое саммари по этому тексту.
Твой ответ должен начинаться с вызова инструмента, а затем, после получения результата, ты должен предоставить саммари.
Пример:
[TOOL_CALL] {"tool": "content_fetcher", "params": {"goal": "https://example.com"}}
"""
        super().__init__(llm_instance, system_prompt, tools_config, log_callback)

    def summarize_source(self, url: str) -> str:
        self.history = []
        task = f"Используй `content_fetcher` для URL '{url}' и напиши саммари."
        
        result_from_tool, finished = self.execute_step(task)
        if finished:
            return f"Агент не смог использовать инструмент для URL: {url}"

        task_for_summary = "Теперь напиши краткое саммари на основе полученного текста."
        summary, _ = self.execute_step(task_for_summary)
        
        return summary