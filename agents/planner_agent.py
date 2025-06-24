# agents/planner_agent.py - ОБУЧЕН ИСПОЛЬЗОВАТЬ ИНДЕКСАТОР КОДА

from .base_agent import BaseAgent
from llama_cpp import Llama
import json
import re

class PlannerAgent(BaseAgent):
    def __init__(self, llm_instance: Llama, log_callback):
        # ИЗМЕНЕНО: Добавлено правило про индексацию кода
        system_prompt = """Ты — опытный тимлид-архитектор. Твоя задача — взять большую цель и разбить ее на четкий, пошаговый план.

**ФОРМАТ ОТВЕТА:**
Твой ответ **ОБЯЗАН** быть JSON-массивом объектов. Каждый объект представляет собой один шаг и должен иметь следующие поля:
- `id`: Уникальный числовой идентификатор шага (начиная с 1).
- `description`: Четкое и краткое описание задачи на этом шаге.
- `status`: Изначальный статус, всегда должен быть `"pending"`.

**ПРАВИЛА РАБОТЫ:**
1.  **ИНДЕКСАЦИЯ КОДА:** Если задача связана с анализом, изменением или рефакторингом существующего кода, твоим **ПЕРВЫМ ШАГОМ** в плане **ВСЕГДА** должен быть вызов инструмента `code_indexer`. Это гарантирует, что у других агентов будет самая свежая информация о проекте.
2.  **ВЗАИМОДЕЙСТВИЕ С ЧЕЛОВЕКОМ:** Если тебе не хватает информации для принятия решения, используй инструмент `ask_human`. Получив ответ, ты **ОБЯЗАН** использовать эту информацию для составления нового, окончательного плана в формате JSON.

**ПРИМЕР (ЗАДАЧА НА КОД):**
Цель: "Добавь в проект функцию для расчета скидки"
Твой ответ:
```json
[
  {
    "id": 1,
    "description": "Вызвать инструмент `code_indexer` для обновления контекста проекта.",
    "status": "pending"
  },
  {
    "id": 2,
    "description": "Проанализировать существующие модули, чтобы найти подходящее место для новой функции.",
    "status": "pending"
  },
  {
    "id": 3,
    "description": "Написать код функции `calculate_discount` в соответствующем файле.",
    "status": "pending"
  }
]
```
"""
        # Инструменты, которые знает планировщик
        planner_tools = {
            "ask_human": {},
            "code_indexer": {}
        }
        super().__init__(llm_instance, system_prompt, planner_tools, log_callback)

    def create_plan(self, goal: str, project_context: str = "") -> list[dict]:
        self.history = []
        
        user_prompt = f"Создай пошаговый план в формате JSON для следующей цели: {goal}"
        if project_context: user_prompt = f"Контекст проекта:\n{project_context}\n\n{user_prompt}"
        
        response, finished = self.execute_step(user_prompt)

        if not finished and "ask_human" in str(response):
            self.log("[PlannerAgent] Агент задал уточняющий вопрос. Ожидание ответа...")
            final_plan_task = "Отлично, теперь на основе этого ответа составь окончательный пошаговый план в формате JSON."
            response, _ = self.execute_step(final_plan_task)

        try:
            match = re.search(r'\[[\s\S]*\]', response)
            if not match:
                raise ValueError("JSON-массив не найден в ответе модели.")
            
            json_part = match.group(0)
            plan = json.loads(json_part)
            
            if isinstance(plan, list) and all(isinstance(item, dict) and 'id' in item and 'description' in item for item in plan):
                return plan
            else:
                self.log("[PlannerAgent] [ERROR] JSON не соответствует требуемой структуре плана.")
                return []
        except (json.JSONDecodeError, ValueError) as e:
            self.log(f"[PlannerAgent] [ERROR] Не удалось извлечь JSON-план из ответа: {e}\nОтвет был: {response}")
            return []
