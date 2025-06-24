# agents/data_extractor_agent.py - ПЕРЕПИСАН ДЛЯ ЛОКАЛЬНОГО ИЗВЛЕЧЕНИЯ ДАННЫХ

from .base_agent import BaseAgent
from llama_cpp import Llama
import urllib.parse

class DataExtractorAgent(BaseAgent):
    def __init__(self, llm_instance: Llama, tools_config: dict, log_callback):
        # У этого агента теперь сложная логика, но нет прямого системного промпта для LLM,
        # так как он управляет другими инструментами и использует LLM на последнем шаге.
        system_prompt = "Ты — AI-ассистент, который извлекает информацию из текста."
        super().__init__(llm_instance, system_prompt, tools_config, log_callback)

    def extract_data(self, url: str, prompt: str) -> str:
        """
        Извлекает структурированные данные с веб-страницы, используя локальные инструменты.
        """
        self.history = []
        self.log(f"[{self.__class__.__name__}] Начинаю извлечение данных с URL: {url}")

        # Шаг 1: Получить HTML-содержимое страницы
        self.log(f"[{self.__class__.__name__}] Шаг 1: Получаю HTML...")
        html_content_result = self._execute_tool("browser_action", {
            "call_path": "page.content",
            "args": [],
            "kwargs": {}
        })
        
        # Предварительно нужно перейти на страницу
        self._execute_tool("browser_action", {"call_path": "page.goto", "args": [url], "kwargs": {}})
        html_content = self._execute_tool("browser_action", {"call_path": "page.content", "args": [], "kwargs": {}})

        if isinstance(html_content, dict) and 'error' in html_content:
            error_msg = f"Ошибка при получении HTML: {html_content.get('details', html_content)}"
            self.log(f"[{self.__class__.__name__}] [ERROR] {error_msg}")
            return error_msg

        # Шаг 2: Преобразовать HTML в чистый Markdown
        self.log(f"[{self.__class__.__name__}] Шаг 2: Конвертирую HTML в Markdown...")
        # Используем data URI, чтобы передать HTML напрямую
        html_data_uri = "data:text/html;charset=utf-8," + urllib.parse.quote(html_content)
        
        markdown_content_result = self._execute_tool("convert_to_markdown", {"uri": html_data_uri})
        
        if isinstance(markdown_content_result, dict) and 'error' in markdown_content_result:
            error_msg = f"Ошибка при конвертации в Markdown: {markdown_content_result.get('details', markdown_content_result)}"
            self.log(f"[{self.__class__.__name__}] [ERROR] {error_msg}")
            return error_msg
        
        # Ожидаем, что результат будет в ключе 'markdown'
        markdown_text = markdown_content_result.get("markdown", str(markdown_content_result))

        # Шаг 3: Извлечь данные из Markdown с помощью LLM
        self.log(f"[{self.__class__.__name__}] Шаг 3: Извлекаю информацию из Markdown с помощью LLM...")
        
        extraction_task = f"""Вот текст, извлеченный и очищенный со страницы {url}:
---
{markdown_text[:4000]}
---

Твоя задача: Внимательно изучи текст и дай точный ответ на следующий запрос:
"{prompt}"
"""
        
        # Используем execute_step для вызова LLM с этой задачей
        final_answer, _ = self.execute_step(extraction_task)
        
        self.log(f"[{self.__class__.__name__}] Извлечение завершено.")
        return final_answer
