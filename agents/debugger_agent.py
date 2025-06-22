# agents/debugger_agent.py - ПЕРЕРАБОТАН В АНАЛИТИКА ОШИБОК

from .base_agent import BaseAgent
from llama_cpp import Llama
import json

class DebuggerAgent(BaseAgent):
    def __init__(self, llm_instance: Llama, tools_config: dict, log_callback):
        # --- Загрузка основного промпта ---
        try:
            with open("prompts/debugger_prompt.md", "r", encoding="utf-8") as f:
                system_prompt = f.read()
        except FileNotFoundError:
            log_callback("[DebuggerAgent] [ERROR] Файл prompts/debugger_prompt.md не найден. Использую запасной промпт.")
            system_prompt = "Ты — отладчик. Анализируй результат запуска кода. Если есть ошибка, опиши ее."

        super().__init__(llm_instance, system_prompt, tools_config, log_callback)

    def run_and_analyze(self, language: str, code_to_run: str) -> str:
        """Запускает код в песочнице и анализирует результат."""
        self.history = []
        self.log(f"[{self.__class__.__name__}] Получил код для запуска на языке '{language}'")
        
        # Шаг 1: Выполнение кода в песочнице
        execution_result = self._execute_tool("code_sandbox", {"language": language, "code": code_to_run})
        
        # Убедимся, что результат - это словарь, как мы ожидаем
        if not isinstance(execution_result, dict):
            return f"ОШИБКА: Не удалось выполнить код. Ответ от песочницы: {execution_result}"

        # Шаг 2: Анализ результата с помощью LLM
        analysis_task = f"""Проанализируй следующий результат выполнения кода и дай заключение согласно протоколу.

Результат:
```json
{json.dumps(execution_result, indent=2)}
```
"""
        # Устанавливаем малое количество токенов для краткого ответа
        original_max_tokens = self.llm.max_tokens
        self.llm.max_tokens = 256
        
        final_verdict, _ = self.execute_step(analysis_task)
        
        self.llm.max_tokens = original_max_tokens
        
        return final_verdict