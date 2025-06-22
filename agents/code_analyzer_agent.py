# agents/code_analyzer_agent.py - ИЗМЕНЕН ПУТЬ ПО УМОЛЧАНИЮ НА 'sandbox'

from .base_agent import BaseAgent
from llama_cpp import Llama
import json
import os
from pathlib import Path

class CodeAnalyzerAgent(BaseAgent):
    def __init__(self, llm_instance: Llama, tools_config: dict, log_callback):
        try:
            with open("prompts/code_analyzer_prompt.md", "r", encoding="utf-8") as f:
                system_prompt = f.read()
        except FileNotFoundError:
            log_callback("[CodeAnalyzerAgent] [ERROR] Файл prompts/code_analyzer_prompt.md не найден. Использую запасной промпт.")
            system_prompt = "Ты — AI-аналитик, эксперт по LSP. Твоя задача — использовать LSP-инструменты для анализа кода."

        super().__init__(llm_instance, system_prompt, tools_config, log_callback)
        self.is_initialized = False

    def analyze_code(self, task: str, project_path: str = "sandbox") -> str:
        """
        Выполняет задачу по анализу кода, при необходимости инициализируя LSP.
        """
        if not self.is_initialized:
            self.log("[CodeAnalyzerAgent] LSP не инициализирован. Выполняю инициализацию...")
            
            abs_path = Path(project_path).resolve()
            root_uri = abs_path.as_uri()
            
            init_params = {"root_uri": root_uri}
            init_result = self._execute_tool("lsp_initialize", init_params)
            
            self.log(f"[CodeAnalyzerAgent] Результат инициализации: {init_result}")
            self.is_initialized = True
            self.history.append({"role": "assistant", "content": "[TOOL_CALL] {\"tool\": \"lsp_initialize\", \"params\": " + json.dumps(init_params) + "}"})
            self.history.append({"role": "tool", "content": json.dumps(init_result)})

        self.log(f"[CodeAnalyzerAgent] Выполняю задачу: {task}")
        result, _ = self.execute_step(task)
        
        return result