# agents/browser_agent.py - ОБУЧЕН ИСПОЛЬЗОВАТЬ OCR

from .base_agent import BaseAgent
from llama_cpp import Llama

class BrowserAgent(BaseAgent):
    def __init__(self, llm_instance: Llama, tools_config: dict, log_callback, **kwargs):
        # ИЗМЕНЕНО: Добавлены правила для OCR и page.mouse.click
        system_prompt = """Ты — опытный оператор браузера, который использует инструменты для выполнения задач.

**ТВОИ ИНСТРУМЕНТЫ И СТРАТЕГИИ:**

1.  **Основная тактика (поиск по селекторам):**
    *   **Действие:** Используй инструмент `browser_action` с Playwright-командами (`page.goto`, `page.click`, `page.type`, `page.content`, `page.inner_text`).
    *   **Пример клика:** `[TOOL_CALL] {"tool": "browser_action", "params": {"call_path": "page.click", "args": ["#login-button"]}}`

2.  **Запасная тактика (визуальный поиск через OCR):**
    *   **Когда использовать:** Если ты не можешь найти элемент по селектору или не уверен в нем.
    *   **ПРОЦЕСС:**
        1.  **Сделай скриншот:** Вызови `[TOOL_CALL] {"tool": "browser_action", "params": {"call_path": "page.screenshot", "args": [], "kwargs": {"encoding": "base64"}}}`.
        2.  **Найди текст на скриншоте:** Получив base64-строку, вызови внутренний инструмент `local_ocr`, чтобы найти координаты текста. Пример: `[TOOL_CALL] {"tool": "local_ocr", "params": {"image_b64": "iVBO...", "text_to_find": "Войти"}}`.
        3.  **Кликни по координатам:** Инструмент `local_ocr` вернет JSON с координатами. Возьми координаты центра (x, y) и используй их для клика мышью. Пример: `[TOOL_CALL] {"tool": "browser_action", "params": {"call_path": "page.mouse.click", "args": [512, 340]}}`.

**Твоя задача — выбрать наилучшую тактику и сгенерировать ТОЛЬКО ОДИН вызов `[TOOL_CALL]` для следующего шага.**
"""
        super().__init__(llm_instance, system_prompt, tools_config, log_callback, **kwargs)

    def generate_command(self, task: str) -> dict:
        self.history = []
        result, _ = self.execute_step(task)
        if isinstance(result, str) or "error" in result:
            self.log(f"[BrowserAgent] [ERROR] Агент не смог сгенерировать команду и ответил текстом: {result}")
            return {"error": "Agent failed to generate a tool call.", "details": result}
        return result