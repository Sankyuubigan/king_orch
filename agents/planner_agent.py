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
```<<<END_FILE>>>

<<<FILE: crews/browser_crew.py>>>
```py
# crews/browser_crew.py - УЛУЧШЕНА ЛОГИКА ПОИСКА URL

import json
from agents.url_search_agent import URLSearchAgent
from agents.browser_agent import BrowserAgent
from agents.html_analyst_agent import HTMLAnalystAgent
from agents.synthesis_agent import SynthesisAgent # <-- Импортируем "Суммаризатора"
from llama_cpp import Llama

class BrowserCrew:
    def __init__(self, llm_instance: Llama, tools_config: dict):
        self.llm = llm_instance
        self.trajectory = []
        self.tools_config = tools_config
        self.update_callback = None

    def _log(self, message):
        self.log_callback(message)
        self.trajectory.append(message)
    
    def _send_update(self, update_type: str, data: any):
        if self.update_callback: self.update_callback({"type": update_type, "data": data})

    def _execute_browser_command(self, agent: BrowserAgent, command_text: str) -> dict:
        self._log(f"[BrowserCrew] Даю команду оператору: '{command_text}'")
        command_json = agent.generate_command(command_text)
        if not isinstance(command_json, dict) or "tool" not in command_json:
            self._log(f"[BrowserCrew] [ERROR] Оператор не смог сгенерировать команду. Ответ: {command_json}")
            return {"error": "Command generation failed", "details": str(command_json)}
        result = agent._execute_tool(command_json.get("tool"), command_json.get("params", {}))
        return result.get('result', result) if isinstance(result, dict) else result

    def _take_and_send_screenshot(self, agent: BrowserAgent):
        self._log("[BrowserCrew] Делаю скриншот...")
        command_json = agent.generate_command("сделай скриншот страницы в формате base64")
        if not isinstance(command_json, dict) or "tool" not in command_json: return
        raw_result = agent._execute_tool(command_json.get("tool"), command_json.get("params", {}))
        if isinstance(raw_result, dict) and 'result' in raw_result and isinstance(raw_result['result'], str):
            self._send_update("browser_screenshot", raw_result['result'])
            self._log("[BrowserCrew] Скриншот отправлен в UI.")

    def run(self, topic: str, log_callback_from_engine, update_callback_from_engine):
        self.log_callback = log_callback_from_engine
        self.update_callback = update_callback_from_engine
        
        url_finder = URLSearchAgent(self.llm, self.tools_config, self._log)
        browser_operator = BrowserAgent(self.llm, self.tools_config, self._log)
        html_analyst = HTMLAnalystAgent(self.llm, self._log)
        # ИЗМЕНЕНО: Инициализируем агента для "очистки" запроса
        extractor = SynthesisAgent(self.llm, self._log)

        # ИЗМЕНЕНО: Используем extractor для извлечения сути запроса
        self._log(f"[BrowserCrew] Извлекаю суть из запроса: '{topic}'...")
        search_goal = extractor.synthesize_report(
            topic="Извлеки из этого текста короткую поисковую фразу, не более 5 слов.",
            summaries=[topic]
        )
        self._log(f"[BrowserCrew] Очищенная цель для поиска: '{search_goal}'")

        task_parts = search_goal.split(" и ", 1)
        navigation_goal = task_parts[0]
        action_goal = task_parts[1] if len(task_parts) > 1 else "Просто осмотрись и верни HTML-код"

        self._log(f"[BrowserCrew] Этап 1: Ищу URL для цели '{navigation_goal}'...")
        target_url = url_finder.find_url(navigation_goal)

        if "Ошибка" in str(target_url) or not str(target_url).startswith("http"):
            error_msg = f"Не удалось найти корректный URL. Результат: {target_url}"
            final_result = {"final_result": error_msg, "trajectory": self.trajectory}
            self._send_update("final_result", final_result); return final_result
        
        self._log(f"[BrowserCrew] URL найден: {target_url}")
        self._log(f"[BrowserCrew] Этап 2: Перехожу по URL...")
        self._execute_browser_command(browser_operator, f"Зайти на сайт {target_url}")
        self._take_and_send_screenshot(browser_operator)
        
        self._log(f"[BrowserCrew] Этап 3: Читаю HTML-содержимое страницы...")
        html_content = self._execute_browser_command(browser_operator, "Получи HTML-код текущей страницы")
        if isinstance(html_content, dict) and "error" in html_content:
            error_msg = f"Ошибка на этапе чтения страницы: {html_content['details']}"
            final_result = {"final_result": error_msg, "trajectory": self.trajectory}
            self._send_update("final_result", final_result); return final_result

        self._log(f"[BrowserCrew] Этап 4: Анализирую HTML, чтобы найти '{action_goal}'...")
        selector = html_analyst.find_selector_in_html(str(html_content), action_goal)
        
        if not selector or "SELECTOR_NOT_FOUND" in selector:
            error_msg = f"Не удалось найти селектор для элемента '{action_goal}'."
            final_result = {"final_result": error_msg, "trajectory": self.trajectory}
            self._send_update("final_result", final_result); return final_result

        self._log(f"[BrowserCrew] Найден селектор: '{selector}'")
        action_command = f"Прочитай текст элемента с селектором '{selector}'"
        if "нажми" in action_goal.lower() or "кликни" in action_goal.lower():
            action_command = f"Кликни на элемент с селектором '{selector}'"
        elif "введи" in action_goal.lower() or "напиши" in action_goal.lower():
            text_to_type = action_goal.split("'")[1] if "'" in action_goal else "test"
            action_command = f"Введи '{text_to_type}' в поле с селектором '{selector}'"

        self._log(f"[BrowserCrew] Этап 5: Выполняю действие...")
        action_result = self._execute_browser_command(browser_operator, action_command)
        self._take_and_send_screenshot(browser_operator)

        final_report = f"Задача '{topic}' выполнена.\n\nРезультат: {action_result}"
        final_result = {"final_result": final_report, "trajectory": self.trajectory}
        self._send_update("final_result", final_result)
        return final_result