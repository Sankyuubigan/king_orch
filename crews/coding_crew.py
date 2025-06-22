# crews/coding_crew.py - ИНТЕГРАЦИЯ РЕТРИВЕРА ДЛЯ ПОЛУЧЕНИЯ КОНТЕКСТА

import json
from agents.planner_agent import PlannerAgent
from agents.code_agent import CodeAgent
from agents.debugger_agent import DebuggerAgent
from agents.code_analyzer_agent import CodeAnalyzerAgent
from agents.code_retriever_agent import CodeRetrieverAgent # <-- Импорт Ретривера

from llama_cpp import Llama

class CodingCrew:
    def __init__(self, llm_instance: Llama, tools_config: dict):
        self.llm = llm_instance
        self.tools_config = tools_config
        self.trajectory = []
        self.update_callback = None
        self.agents = {}

    def _log(self, message):
        self.log_callback(message)
        self.trajectory.append(message)

    def _send_update(self, update_type: str, data: any):
        if self.update_callback:
            self.update_callback({"type": update_type, "data": data})

    def _handle_agent_call(self, agent_name: str, task: str):
        self._log(f"[CrewRouter] Поступил вызов для агента '{agent_name}' с задачей: {task[:60]}...")
        
        agent_instance = self.agents.get(agent_name)
        if not agent_instance:
            error_msg = f"Ошибка маршрутизации: Агент '{agent_name}' не найден."
            self._log(f"[CrewRouter] [ERROR] {error_msg}")
            return error_msg

        # Маршрутизация вызовов к соответствующим методам агентов
        if agent_name == "CodeAnalyzerAgent":
            return agent_instance.analyze_code(task, project_path="sandbox")
        elif agent_name == "CodeRetrieverAgent":
            return agent_instance.retrieve_code_snippets(task)
        elif agent_name == "DebuggerAgent":
            lang, code = "python", task 
            return agent_instance.run_and_analyze(lang, code)
        else:
            return agent_instance.work_on_task(task)

    def run(self, topic: str, log_callback_from_engine, update_callback_from_engine):
        self.log_callback = log_callback_from_engine
        self.update_callback = update_callback_from_engine
        
        # --- Инициализация всех агентов команды ---
        self.agents = {
            "PlannerAgent": PlannerAgent(self.llm, self._log),
            "CodeAgent": CodeAgent(self.llm, self.tools_config, self._log, agent_router=self._handle_agent_call),
            "DebuggerAgent": DebuggerAgent(self.llm, self.tools_config, self._log, agent_router=self._handle_agent_call),
            "CodeAnalyzerAgent": CodeAnalyzerAgent(self.llm, self.tools_config, self._log),
            "CodeRetrieverAgent": CodeRetrieverAgent(self.llm, self.tools_config, self._log)
        }
        
        planner = self.agents["PlannerAgent"]
        coder = self.agents["CodeAgent"]

        self._log(f"[Crew] Получена задача: '{topic}'. Начинаю планирование...")
        plan = planner.create_plan(topic)
        if not plan:
            self._send_update("final_result", {"final_result": "Планировщик не смог составить план.", "trajectory": self.trajectory})
            return
        
        self._log("[Crew] План составлен.")
        self._send_update("plan", plan)

        for step in plan:
            task_id = step['id']
            task_description = step['description']
            
            self._log(f"\n[Crew] --- Шаг {task_id}: {task_description} ---")
            self._send_update("status_update", {"id": task_id, "status": "running"})

            try:
                # Просто передаем задачу ведущему инженеру (CodeAgent)
                # Он сам решит, когда и какого "коллегу" (Retriever или Analyzer) вызвать
                result = coder.work_on_task(task_description)
                
                if "ошибка" in str(result).lower() or "error" in str(result).lower():
                     self._send_update("status_update", {"id": task_id, "status": "failed"})
                else:
                     self._send_update("status_update", {"id": task_id, "status": "done"})

            except Exception as e:
                self._log(f"[Crew] [FATAL ERROR] Критическая ошибка на шаге {task_id}: {e}")
                self._send_update("status_update", {"id": task_id, "status": "failed"})
                self._send_update("final_result", {"final_result": f"Критическая ошибка: {e}", "trajectory": self.trajectory})
                return

        final_report = "Все шаги плана выполнены."
        self._log(f"[Crew] Завершено.")
        self._send_update("final_result", {"final_result": final_report, "trajectory": self.trajectory})