# crews/docs_crew.py - ИСПРАВЛЕНА СИГНАТУРА МЕТОДА RUN

from agents.docs_expert_agent import DocsExpertAgent
from llama_cpp import Llama

class DocsCrew:
    def __init__(self, llm_instance: Llama, tools_config: dict):
        self.llm = llm_instance
        self.trajectory = []
        self.tools_config = tools_config
        self.update_callback = None # Добавлено для консистентности

    def _log(self, message):
        self.log_callback(message)
        self.trajectory.append(message)

    # ИЗМЕНЕНО: Добавлен третий аргумент `update_callback_from_engine` для унификации
    def run(self, topic: str, log_callback_from_engine, update_callback_from_engine=None):
        self.log_callback = log_callback_from_engine
        self.update_callback = update_callback_from_engine # Сохраняем, даже если не используем
        
        docs_expert = DocsExpertAgent(self.llm, self.tools_config, self._log)

        self._log(f"[DocsCrew] Получен вопрос по документации: '{topic}'...")
        answer = docs_expert.answer_from_docs(topic)
        
        self._log("[DocsCrew] Ответ получен.")
        
        # Возвращаем результат в стандартном формате
        final_result = {"final_result": answer, "trajectory": self.trajectory}
        
        # Используем update_callback, если он есть, для отправки финального результата
        if self.update_callback:
            self.update_callback({"type": "final_result", "data": final_result})
        
        return final_result