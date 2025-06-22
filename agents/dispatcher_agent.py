# agents/dispatcher_agent.py - НАУЧИЛСЯ ВЫБИРАТЬ МОДЕЛЬ

from .base_agent import BaseAgent
from llama_cpp import Llama
import json
import re

class DispatcherAgent(BaseAgent):
    def __init__(self, llm_instance: Llama, log_callback):
        system_prompt = """Твоя задача — классифицировать запрос пользователя и определить, какая команда агентов и какая AI-модель должны его обработать.

**МОДЕЛИ:**
- `default`: Универсальная модель для общения, исследований, браузинга.
- `coding`: Специализированная модель, заточенная под написание, анализ и рефакторинг кода.

**КОМАНДЫ:**
- `coding`: Запрос связан с написанием или изменением кода. **ИСПОЛЬЗУЙ `coding` МОДЕЛЬ.**
- `browsing`: Запрос требует действий в браузере. **ИСПОЛЬЗУЙ `default` МОДЕЛЬ.**
- `documentation_query`: Запрос к документации. **ИСПОЛЬЗУЙ `default` МОДЕЛЬ.**
- `research`: Общий поиск информации. **ИСПОЛЬЗУЙ `default` МОДЕЛЬ.**
- `general_conversation`: Простое общение. **ИСПОЛЬЗУЙ `default` МОДЕЛЬ.**

Твой ответ **ОБЯЗАН** быть JSON-объектом с двумя ключами:
- `crew_type`: Название команды (одно из списка выше).
- `model_key`: Ключ модели (`default` или `coding`).

**ПРИМЕРЫ:**
Запрос: "привет"
Твой ответ:
{"crew_type": "general_conversation", "model_key": "default"}

Запрос: "напиши скрипт на питоне для парсинга сайта"
Твой ответ:
{"crew_type": "coding", "model_key": "coding"}

Запрос: "зайди на vk.com/durov"
Твой ответ:
{"crew_type": "browsing", "model_key": "default"}
"""
        super().__init__(llm_instance, system_prompt, {}, log_callback)

    def choose_crew_and_model(self, goal: str) -> dict:
        """Определяет команду и необходимую модель, возвращая словарь."""
        self.history = []
        self.llm.max_tokens = 100 # Увеличим лимит для JSON
        
        response, _ = self.execute_step(f"Классифицируй этот запрос: \"{goal}\"")
        
        self.llm.max_tokens = 1024 # Возвращаем стандартное значение
        
        try:
            # Ищем JSON объект в ответе
            match = re.search(r'\{[\s\S]*\}', response)
            if not match:
                raise ValueError("JSON-объект не найден в ответе модели.")
            
            json_part = match.group(0)
            result = json.loads(json_part)
            
            # Проверяем наличие ключей и возвращаем с sane defaults
            return {
                "crew_type": result.get("crew_type", "general_conversation"),
                "model_key": result.get("model_key", "default")
            }
        except (json.JSONDecodeError, ValueError) as e:
            self.log(f"[DispatcherAgent] [ERROR] Не удалось извлечь JSON: {e}\nОтвет был: {response}")
            # Возвращаем дефолтное значение в случае ошибки
            return {"crew_type": "general_conversation", "model_key": "default"}