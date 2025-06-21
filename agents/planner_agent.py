# agents/planner_agent.py - ТЕПЕРЬ УЧИТЫВАЕТ КОНТЕКСТ ПРОЕКТА

from .base_agent import BaseAgent
from llama_cpp import Llama

class PlannerAgent(BaseAgent):
    def __init__(self, llm_instance: Llama, log_callback):
        system_prompt = """Ты — опытный тимлид-архитектор. Твоя единственная задача — взять большую цель от пользователя и разбить ее на четкий, пошаговый план в формате numerated list.
Каждый шаг должен быть простым, атомарным действием, которое может выполнить другой агент.
Если тебе предоставлен контекст проекта (список файлов), обязательно учитывай его. Например, если файл уже существует, не создавай его заново, а предлагай изменить.

Пример:
Цель: Создай скрипт, который скачивает главную страницу google.com и сохраняет ее в index.html.
Твой план:
1. Создать файл `main.py` с кодом для скачивания страницы с помощью библиотеки requests.
2. Создать файл `requirements.txt` и записать в него 'requests'.
3. Запустить команду `pip install -r requirements.txt` для установки зависимостей.
4. Запустить скрипт `python main.py`.
5. Проверить, что файл `index.html` был успешно создан.

Не пиши ничего, кроме плана. Только нумерованный список."""
        super().__init__(llm_instance, system_prompt, {}, log_callback)

    def create_plan(self, goal: str, project_context: str = "") -> list[str]:
        self.history = []
        
        # Формируем промпт с учетом контекста проекта, если он есть
        user_prompt = f"Создай пошаговый план для следующей цели: {goal}"
        if project_context and project_context.strip() and "не удалось" not in project_context.lower():
            user_prompt = f"Контекст проекта (содержимое директории 'workspace'):\n---\n{project_context}\n---\n\n{user_prompt}"
        
        plan_text, _ = self.execute_step(user_prompt)
        return [step.strip() for step in plan_text.split('\n') if step.strip() and step.strip()[0].isdigit()]