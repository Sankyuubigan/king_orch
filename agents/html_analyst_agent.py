# agents/html_analyst_agent.py - ИСПРАВЛЕНО: ПРОМПТ ВЫНЕСЕН В ФАЙЛ

from .base_agent import BaseAgent
from llama_cpp import Llama

class HTMLAnalystAgent(BaseAgent):
    def __init__(self, llm_instance: Llama, log_callback):
        # --- Загрузка основного промпта ---
        try:
            with open("prompts/html_analyst_prompt.md", "r", encoding="utf-8") as f:
                system_prompt = f.read()
        except FileNotFoundError:
            log_callback("[HTMLAnalystAgent] [ERROR] Файл prompts/html_analyst_prompt.md не найден. Использую запасной промпт.")
            system_prompt = "Ты — AI-аналитик, эксперт по HTML и CSS. Твоя задача — найти CSS-селектор для элемента по HTML-коду и запросу. Возвращай только селектор."

        # У этого агента нет инструментов, поэтому tools_config пустой
        super().__init__(llm_instance, system_prompt, {}, log_callback)

    def find_selector_in_html(self, html_content: str, query: str) -> str:
        """
        Находит CSS-селектор в HTML по текстовому запросу.
        """
        self.history = []
        # Уменьшаем объем HTML, чтобы не превысить контекст модели
        html_snippet = html_content[:8000]
        
        task = f"""Вот HTML-код страницы (или его фрагмент):
```html
{html_snippet}
```

А вот что нужно найти: '{query}'.

Проанализируй HTML и верни самый точный CSS-селектор.
"""
        
        # Устанавливаем малое количество токенов для ответа, т.к. нам нужен только селектор
        original_max_tokens = self.llm.max_tokens
        self.llm.max_tokens = 50
        
        selector, finished = self.execute_step(task)
        
        # Возвращаем исходное значение max_tokens
        self.llm.max_tokens = original_max_tokens

        if finished:
            # Убираем лишние символы и кавычки, которые модель может добавить
            clean_selector = selector.strip().replace('`', '').replace('"', '').replace("'", "")
            return clean_selector
        else:
            # Если агент попытался использовать инструмент (чего не должно быть), возвращаем ошибку
            return "SELECTOR_NOT_FOUND"