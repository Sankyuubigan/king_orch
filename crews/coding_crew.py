# crews/coding_crew.py - УБРАНА ЛОКАЛЬНАЯ ЗАГРУЗКА КОНФИГА

import json
import requests
from agents.planner_agent import PlannerAgent
from agents.code_agent import CodeAgent
from agents.debugger_agent import DebuggerAgent
from llama_cpp import Llama

class CodingCrew:
    def __init__(self, llm_instance: Llama, tools_config: dict):
        self.llm = llm_instance
        self.trajectory = []
        # Конфиг теперь передается снаружи, а не загружается здесь
        self.tools_config = tools_config

    def _log(self, message):
        self.log_callback(message)
        self.trajectory.append(message)

    def _get_project_context(self) -> str:
        """Использует инструмент file_lister для получения структуры проекта."""
        try:
            tool_info = self.tools_config["file_lister"]
            url = tool_info["url"]
            response = requests.get(url, params={"path": "."}, timeout=10)
            response.raise_for_status()
            return response.json().get("result", "Не удалось получить список файлов.")
        except Exception as e:
            self._log(f"[Crew] [WARNING] Не удалось получить контекст проекта: {e}")
            return "Не удалось получить список файлов."

    def run(self, topic: str, log_callback_from_engine):
        self.log_callback = log_callback_from_engine
        
        planner = PlannerAgent(self.llm, self._log)
        coder = CodeAgent(self.llm, self.tools_config, self._log)
        debugger = DebuggerAgent(self.llm, self.tools_config, self._log)

        self._log("[Crew] Получаю контекст проекта...")
        project_context = self._get_project_context()
        self._log(f"[Crew] Контекст получен:\n{project_context}")

        self._log(f"[Crew] Получена задача: '{topic}'. Начинаю планирование...")
        plan = planner.create_plan(topic, project_context)
        if not plan:
            final_report = "Планировщик не смог составить план."
            self._log(f"[Crew] [ERROR] {final_report}")
            return {"final_result": final_report, "trajectory": self.trajectory}
        
        self._log("[Crew] План составлен:")
        for i, step in enumerate(plan): self._log(f"  {i+1}. {step}")

        self._log("[Crew] Начинаю выполнение плана...")
        MAX_CORRECTION_ATTEMPTS = 2 

        for i, task in enumerate(plan):
            self._log(f"\n[Crew] --- Шаг {i+1}: {task} ---")
            
            if task.lower().startswith("запустить") or task.lower().startswith("проверить"):
                command_to_run = task.split('`', 2)[1] if '`' in task else task
                
                result = debugger.run_and_debug(command_to_run)
                
                correction_attempts = 0
                while "обнаружена ошибка" in result.lower() and correction_attempts < MAX_CORRECTION_ATTEMPTS:
                    correction_attempts += 1
                    self._log(f"[Crew] [САМОКОРРЕКЦИЯ] Ошибка на шаге {i+1}. Попытка исправления #{correction_attempts}...")
                    
                    correction_task = f"Предыдущая команда '{command_to_run}' провалилась. Вот лог ошибки:\n{result}\nПроанализируй ошибку и исправь код в соответствующих файлах, чтобы решить эту проблему."
                    
                    self._log(f"[Crew] [САМОКОРРЕКЦИЯ] Новая задача для CodeAgent: {correction_task[:150]}...")
                    coder.work_on_task(correction_task)
                    
                    self._log(f"[Crew] [САМОКОРРЕКЦИЯ] Повторный запуск команды '{command_to_run}' для проверки исправления.")
                    result = debugger.run_and_debug(command_to_run)

                if "обнаружена ошибка" in result.lower():
                    final_report = f"Не удалось исправить ошибку на шаге '{task}' после {MAX_CORRECTION_ATTEMPTS} попыток. Выполнение прервано."
                    self._log(f"[Crew] [ERROR] {final_report}")
                    return {"final_result": final_report, "trajectory": self.trajectory}

            else:
                result = coder.work_on_task(task)
            
            self._log(f"[Crew] Результат шага: {result}")
        
        final_report = "Все шаги плана выполнены. Проверьте рабочую директорию 'workspace' для результатов."
        self._log(f"[Crew] Завершено. {final_report}")
        
        return {"final_result": final_report, "trajectory": self.trajectory}