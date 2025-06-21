# agents/debugger_agent.py - УЛУЧШЕНА ЛОГИКА ПРОВЕРКИ

from .base_agent import BaseAgent
from llama_cpp import Llama

class DebuggerAgent(BaseAgent):
    def __init__(self, llm_instance: Llama, tools_config: dict, log_callback):
        system_prompt = """Ты — скрупулезный QA-инженер. Твоя задача — взять команду для запуска, выполнить её и проанализировать результат.
Если есть ошибка (exit code не 0 или что-то в stderr), четко опиши проблему. Если все хорошо, так и скажи.
Используй инструмент `code_executor` для выполнения команд.
Твой ответ ДОЛЖЕН быть вызовом инструмента: [TOOL_CALL] {"tool": "code_executor", "params": {"command": "команда для выполнения"}}"""
        super().__init__(llm_instance, system_prompt, tools_config, log_callback)

    def run_and_debug(self, command_to_run: str) -> str:
        self.history = []
        self.log(f"[{self.__class__.__name__}] Получил команду для запуска: {command_to_run}")
        result = self._execute_tool("code_executor", {"command": command_to_run})
        
        # Успех — это код выхода 0 и ПУСТОЙ stderr.
        # mcp_code_runner форматирует пустой stderr как "--- STDERR ----\n\n".
        # Эта проверка стала более надежной.
        is_success = "Exit Code: 0" in result and "--- STDERR ----\n\n" in result

        if is_success:
             return f"Команда выполнена успешно. Вывод:\n{result}"
        else:
             return f"Обнаружена ошибка при выполнении. Вывод:\n{result}"