
# crews/browser_crew.py - ПЕРЕДАЕТ РОУТЕР ВНУТРЕННИХ ИНСТРУМЕНТОВ В АГЕНТЫ

import json
from agents.url_search_agent import URLSearchAgent
from agents.browser_agent import BrowserAgent
from agents.html_analyst_agent import HTMLAnalystAgent
from agents.synthesis_agent import SynthesisAgent
from llama_cpp import Llama

class BrowserCrew:
    # ИЗМЕНЕНО: Принимает весь объект движка
    def __init__(self, llm_instance: Llama, tools_config: dict, engine):
        self.llm = llm_instance
        self.tools_config = tools_config
        self.engine = engine # Сохраняем ссылку на движок
        self.trajectory = []
        self.update_callback = None

    def _log(self, message):
        if hasattr(self, 'log_callback') and self.log_callback:
            self.log_callback(message)
        self.trajectory.append(message)
    
    def _send_update(self, update_type: str, data: any):
        if self.update_callback: self.update_callback({"type": update_type, "data": data})

    def _execute_browser_command(self, agent: BrowserAgent, command_text: str) -> dict:
        self._log(f"[BrowserCrew] Даю команду оператору: '{command_text}'")
        # Агент сам вызовет либо HTTP, либо внутренний инструмент
        result = agent.generate_command(command_text)
        return result

    def _take_and_send_screenshot(self, agent: BrowserAgent):
        self._log("[BrowserCrew] Делаю скриншот...")
        command_json = agent.generate_command("сделай скриншот страницы в формате base64")
        if command_json and command_json.get("tool"):
             raw_result = agent._execute_tool(command_json["tool"], command_json.get("params", {}))
             if isinstance(raw_result, dict) and 'result' in raw_result and isinstance(raw_result['result'], str):
                 self._send_update("browser_screenshot", raw_result['result'])
                 self._log("[BrowserCrew] Скриншот отправлен в UI.")

    def run(self, topic: str, log_callback_from_engine, update_callback_from_engine):
        self.log_callback = log_callback_from_engine
        self.update_callback = update_callback_from_engine
        
        # Передаем internal_router в агенты
        agent_kwargs = {"internal_router": self.engine.route_internal_call}
        url_finder = URLSearchAgent(self.llm, self.tools_config, self._log, **agent_kwargs)
        browser_operator = BrowserAgent(self.llm, self.tools_config, self._log, **agent_kwargs)
        html_analyst = HTMLAnalystAgent(self.llm, self._log, **agent_kwargs)
        extractor = SynthesisAgent(self.llm, self._log, **agent_kwargs)

        self._log(f"[BrowserCrew] Извлекаю суть из запроса: '{topic}'...")
        search_goal = extractor.work_on_task(f"Из этого текста '{topic}' извлеки короткую поисковую фразу, не более 5 слов.")
        self._log(f"[BrowserCrew] Очищенная цель для поиска: '{search_goal}'")
        
        task_parts = search_goal.split(" и ", 1)
        navigation_goal, action_goal = task_parts[0], task_parts[1] if len(task_parts) > 1 else "Просто осмотрись"

        target_url = url_finder.work_on_task(f"Найди URL для '{navigation_goal}'")

        if "Ошибка" in str(target_url) or not str(target_url).startswith("http"):
            error_msg = f"Не удалось найти корректный URL. Результат: {target_url}"
            final_result = {"final_result": error_msg, "trajectory": self.trajectory}
            self._send_update("final_result", final_result); return final_result
        
        self._execute_browser_command(browser_operator, f"Зайти на сайт {target_url}")
        self._take_and_send_screenshot(browser_operator)
        
        final_report = f"Задача '{topic}' выполнена.\n\n"
        if "осмотрись" not in action_goal:
            final_result = self._execute_browser_command(browser_operator, action_goal)
            self._take_and_send_screenshot(browser_operator)
            final_report += f"Результат: {final_result}"
        else:
            final_report += "Осмотр завершен."

        self._log(f"[BrowserCrew] Завершено.")
        final_result_data = {"final_result": final_report, "trajectory": self.trajectory}
        self._send_update("final_result", final_result_data)
        return final_result_data
