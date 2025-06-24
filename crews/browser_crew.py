# crews/browser_crew.py - ПОВЫШЕНА НАДЕЖНОСТЬ, ВСЕГДА ВОЗВРАЩАЕТ РЕЗУЛЬТАТ

import json
from agents.url_search_agent import URLSearchAgent
from agents.browser_agent import BrowserAgent
from agents.html_analyst_agent import HTMLAnalystAgent
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
        """Отправляет структурированное сообщение в UI."""
        if self.update_callback:
            self.update_callback({"type": update_type, "data": data})

    def _execute_browser_command(self, agent: BrowserAgent, command_text: str) -> dict:
        """Генерирует и выполняет команду браузера, обрабатывая результат."""
        self._log(f"[BrowserCrew] Даю команду оператору: '{command_text}'")
        command_json = agent.generate_command(command_text)
        
        if not isinstance(command_json, dict) or "tool" not in command_json:
            self._log(f"[BrowserCrew] [ERROR] Оператор не смог сгенерировать команду. Ответ: {command_json}")
            return {"error": "Command generation failed", "details": str(command_json)}

        result = agent._execute_tool(command_json.get("tool"), command_json.get("params", {}))
        
        if isinstance(result, dict) and 'result' in result:
            return result['result']
        return result

    def _take_and_send_screenshot(self, agent: BrowserAgent):
        """Запрашивает скриншот у оператора и отправляет его в UI."""
        self._log("[BrowserCrew] Делаю скриншот...")
        screenshot_command = "сделай скриншот страницы в формате base64"
        
        command_json = agent.generate_command(screenshot_command)
        if not isinstance(command_json, dict) or "tool" not in command_json:
            self._log(f"[BrowserCrew] [ERROR] Не удалось сгенерировать команду для скриншота.")
            return

        raw_result = agent._execute_tool(command_json.get("tool"), command_json.get("params", {}))
        
        if isinstance(raw_result, dict) and 'result' in raw_result:
            screenshot_b64 = raw_result['result']
            if isinstance(screenshot_b64, str) and len(screenshot_b64) > 100:
                 self._send_update("browser_screenshot", screenshot_b64)
                 self._log("[BrowserCrew] Скриншот отправлен в UI.")
            else:
                 self._log(f"[BrowserCrew] [WARNING] Получен некорректный результат для скриншота.")
        else:
            self._log(f"[BrowserCrew] [WARNING] Не удалось получить результат скриншота. Ответ: {raw_result}")


    def run(self, topic: str, log_callback_from_engine, update_callback_from_engine):
        self.log_callback = log_callback_from_engine
        self.update_callback = update_callback_from_engine
        
        url_finder = URLSearchAgent(self.llm, self.tools_config, self._log)
        browser_operator = BrowserAgent(self.llm, self.tools_config, self._log)
        html_analyst = HTMLAnalystAgent(self.llm, self._log)

        # ИЗМЕНЕНО: Упрощаем задачу для URL-поисковика, убирая лишний текст
        task_parts = topic.split(" и ", 1)
        navigation_goal = task_parts[0]
        action_goal = task_parts[1] if len(task_parts) > 1 else "Просто осмотрись и верни HTML-код"

        self._log(f"[BrowserCrew] Этап 1: Ищу URL для цели '{navigation_goal}'...")
        target_url = url_finder.find_url(navigation_goal)

        # ИЗМЕНЕНО: Теперь в случае ошибки мы возвращаем словарь, а не None
        if "Ошибка" in str(target_url) or not str(target_url).startswith("http"):
            error_msg = f"Не удалось найти корректный URL. Результат: {target_url}"
            self._log(f"[BrowserCrew] [ERROR] {error_msg}")
            final_result = {"final_result": error_msg, "trajectory": self.trajectory}
            self._send_update("final_result", final_result)
            return final_result
        
        self._log(f"[BrowserCrew] URL найден: {target_url}")

        self._log(f"[BrowserCrew] Этап 2: Перехожу по URL...")
        self._execute_browser_command(browser_operator, f"Зайти на сайт {target_url}")
        self._take_and_send_screenshot(browser_operator)
        
        self._log(f"[BrowserCrew] Этап 3: Читаю HTML-содержимое страницы...")
        html_content = self._execute_browser_command(browser_operator, "Получи HTML-код текущей страницы")
        if isinstance(html_content, dict) and "error" in html_content:
            error_msg = f"Ошибка на этапе чтения страницы: {html_content['details']}"
            final_result = {"final_result": error_msg, "trajectory": self.trajectory}
            self._send_update("final_result", final_result)
            return final_result

        self._log(f"[BrowserCrew] Этап 4: Анализирую HTML, чтобы найти '{action_goal}'...")
        selector = html_analyst.find_selector_in_html(str(html_content), action_goal)
        
        if not selector or "SELECTOR_NOT_FOUND" in selector:
            error_msg = f"Не удалось найти селектор для элемента '{action_goal}'. Возвращаю HTML-код."
            self._log(f"[BrowserCrew] [WARNING] {error_msg}")
            final_report = f"{error_msg}\n\n{str(html_content)[:2000]}..."
            final_result = {"final_result": final_report, "trajectory": self.trajectory}
            self._send_update("final_result", final_result)
            return final_result

        self._log(f"[BrowserCrew] Найден селектор: '{selector}'")

        action_command = ""
        if "прочитай" in action_goal.lower() or "найди текст" in action_goal.lower():
            action_command = f"Прочитай текст элемента с селектором '{selector}'"
        elif "нажми" in action_goal.lower() or "кликни" in action_goal.lower():
            action_command = f"Кликни на элемент с селектором '{selector}'"
        elif "введи" in action_goal.lower() or "напиши" in action_goal.lower():
            text_to_type = action_goal.split("'")[1] if "'" in action_goal else "test"
            action_command = f"Введи '{text_to_type}' в поле с селектором '{selector}'"
        else:
            action_command = f"Прочитай текст элемента с селектором '{selector}'"

        self._log(f"[BrowserCrew] Этап 5: Выполняю действие...")
        action_result = self._execute_browser_command(browser_operator, action_command)
        self._take_and_send_screenshot(browser_operator)

        final_report = f"Задача '{topic}' выполнена.\n\nРезультат: {action_result}"
        self._log(f"[BrowserCrew] Завершено.")
        final_result = {"final_result": final_report, "trajectory": self.trajectory}
        self._send_update("final_result", final_result)
        return final_result