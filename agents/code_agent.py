# agents/code_agent.py - ОБНОВЛЕН С ЗАПРЕТОМ НА ИЗМЕНЕНИЕ СВЯЩЕННЫХ ФАЙЛОВ

from .base_agent import BaseAgent
from llama_cpp import Llama
import os

class CodeAgent(BaseAgent):
    def __init__(self, llm_instance: Llama, tools_config: dict, log_callback):
        
        # --- Загрузка священных файлов ---
        sacred_files_content = ""
        try:
            with open("SACRED_FILES.md", "r", encoding="utf-8") as f:
                sacred_files_content = f.read()
        except FileNotFoundError:
            log_callback("[CodeAgent] [WARNING] Файл SACRED_FILES.md не найден.")

        # --- Загрузка основного промпта ---
        try:
            with open("prompts/coding_prompt.md", "r", encoding="utf-8") as f:
                reliability_prompt = f.read()
        except FileNotFoundError:
            log_callback("[CodeAgent] [ERROR] Файл prompts/coding_prompt.md не найден. Использую запасной промпт.")
            reliability_prompt = "Ты — AI-ассистент, инженер-программист. Твоя главная и единственная ценность — НАДЕЖНОСТЬ."

        # --- Формирование финального системного промпта ---
        system_prompt = (
            f"{reliability_prompt}\n\n"
            "--- ПРОТОКОЛ БЕЗОПАСНОСТИ ---\n"
            f"{sacred_files_content}\n"
            "ТЫ ОБЯЗАН СЛЕДОВАТЬ ЭТИМ ЗАПРЕТАМ.\n"
            "--------------------------\n\n"
            "Твой ответ ДОЛЖЕН быть либо мыслью, либо вызовом инструмента в формате: "
            '[TOOL_CALL] {"tool": "название", "params": {"ключ": "значение"}}.\n'
        )
        
        super().__init__(llm_instance, system_prompt, tools_config, log_callback)

    def work_on_task(self, task: str) -> str:
        self.history = []
        result, _ = self.execute_step(task)
        return result