# crews/docs_crew.py - НОВАЯ КОМАНДА ДЛЯ РАБОТЫ С ДОКУМЕНТАЦИЕЙ

from agents.docs_expert_agent import DocsExpertAgent
from llama_cpp import Llama

class DocsCrew:
    def __init__(self, llm_instance: Llama, tools_config: dict):
        self.llm = llm_instance
        self.trajectory = []
        self.tools_config = tools_config

    def _log(self, message):
        self.log_callback(message)
        self.trajectory.append(message)

    def run(self, topic: str, log_callback_from_engine):
        self.log_callback = log_callback_from_engine
        
        docs_expert = DocsExpertAgent(self.llm, self.tools_config, self._log)

        self._log(f"[DocsCrew] Получен вопрос по документации: '{topic}'...")
        answer = docs_expert.answer_from_docs(topic)
        
        self._log("[DocsCrew] Ответ получен.")
        return {"final_result": answer, "trajectory": self.trajectory}