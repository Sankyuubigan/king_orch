# agents/synthesis_agent.py - Принимает llm_instance

from .base_agent import BaseAgent
from llama_cpp import Llama

class SynthesisAgent(BaseAgent):
    def __init__(self, llm_instance: Llama, log_callback):
        system_prompt = """Ты — главный редактор и старший аналитик. Тебе предоставлены несколько отчетов от твоих младших сотрудников.
Твоя задача — изучить их все, сравнить, найти общие темы и противоречия, и написать на их основе один финальный, всеобъемлющий и структурированный отчет для конечного пользователя.
Не используй инструменты. Твоя работа — только анализ и написание текста. Отвечай развернуто и по существу."""
        super().__init__(llm_instance, system_prompt, {}, log_callback)

    def synthesize_report(self, topic: str, summaries: list[str]) -> str:
        self.history = []
        
        summaries_text = "\n\n---\n\n".join(f"Отчет из источника #{i+1}:\n{summary}" for i, summary in enumerate(summaries))
        
        task = f"Тема исследования: '{topic}'.\n\nВот отчеты от младших аналитиков:\n{summaries_text}\n\nНапиши на их основе финальный, сводный отчет."
        
        final_report, _ = self.execute_step(task)
        return final_report