# engine.py - ВЕРСИЯ С ПОДДЕРЖКОЙ НЕСКОЛЬКИХ MCP-ИНСТРУМЕНТОВ

import os
import gc
import json
import traceback
import requests
from llama_cpp import Llama

TOOL_CALL_MARKER = "[TOOL_CALL]"

class OrchestratorEngine:
    def __init__(self, log_callback):
        self.log = log_callback
        self.MODELS_DIR = r"D:\nn\models"
        self.llm = None
        self.model_name = None
        self.history = []
        self.tools = {}
        
        # Загрузка конфигурации инструментов
        try:
            with open("tools_config.json", "r", encoding="utf-8") as f:
                self.tools = json.load(f)
            self.log(f"[Engine] Загружены инструменты: {', '.join(self.tools.keys())}")
        except Exception as e:
            self.log(f"[ERROR] Не удалось загрузить tools_config.json: {e}")

        self.system_prompt_template = ""
        try:
            with open("system_prompt.md", "r", encoding="utf-8") as f:
                self.system_prompt_template = f.read()
            self.log("[Engine] Шаблон системного промпта загружен.")
        except FileNotFoundError:
            self.log("[ERROR] Файл system_prompt.md не найден!")
            self.system_prompt_template = "Ты — полезный ассистент."
        
        self.log("[Engine] Движок инициализирован.")

    def _get_dynamic_system_prompt(self):
        if not self.tools or not self.system_prompt_template:
            return "Ты — полезный ассистент."
        
        tools_description = "\n\n### ДОСТУПНЫЕ ИНСТРУМЕНТЫ ###\n"
        for name, details in self.tools.items():
            tools_description += f'- **`{name}`**: {details["description"]}\n'
            
        return self.system_prompt_template + tools_description


    def get_available_models(self):
        try:
            return [f for f in os.listdir(self.MODELS_DIR) if f.endswith('.gguf')]
        except FileNotFoundError:
            self.log(f"[ERROR] Папка с моделями не найдена: {self.MODELS_DIR}")
            return []

    def _create_chat_prompt(self):
        system_prompt = self._get_dynamic_system_prompt()
        system_message = {"role": "system", "content": system_prompt}
        full_history = [system_message] + self.history
        prompt_str = ""
        for message in full_history:
            if message.get("role") == "tool_result":
                 prompt_str += f"Результат от инструмента:\n{message['content']}\n"
            else:
                 prompt_str += f"<|im_start|>{message['role']}\n{message['content']}<|im_end|>\n"
        prompt_str += "<|im_start|>assistant\n"
        return prompt_str

    def get_current_token_count(self):
        if not self.llm or not self.history: return 0
        tokens = self.llm.tokenize(self._create_chat_prompt().encode("utf-8"))
        return len(tokens)

    def load_model(self, model_name_to_load):
        if not model_name_to_load: return False
        self.unload_model()
        model_path = os.path.join(self.MODELS_DIR, model_name_to_load)
        self.log(f"[INFO] Загрузка модели: {model_name_to_load}")
        try:
            self.llm = Llama(model_path=model_path, n_gpu_layers=-1, n_ctx=4096, verbose=False)
            self.model_name = model_name_to_load
            self.log(f"[SUCCESS] Модель '{self.model_name}' загружена!")
            return True
        except Exception:
            self.log(f"[ERROR] ПРОВАЛ ЗАГРУЗКИ МОДЕЛИ!\n{traceback.format_exc()}")
            self.llm = None; self.model_name = None
            return False

    def unload_model(self):
        if self.llm is None: return
        self.log(f"[INFO] Выгрузка модели '{self.model_name}'...")
        self.llm = None; self.model_name = None; self.history = []
        gc.collect()
        self.log("[INFO] Модель выгружена.")

    def get_response(self, user_prompt):
        if self.llm is None: return {"status": "error", "content": "Модель не загружена."}
        self.history.append({"role": "user", "content": user_prompt})
        
        try:
            full_prompt = self._create_chat_prompt()
            output = self.llm(prompt=full_prompt, max_tokens=512, stop=["<|im_end|>"])
            model_response_text = output['choices'][0]['text'].strip()

            if not model_response_text:
                self.log("[ERROR] Модель вернула пустой ответ. Очистка истории.")
                if self.history and self.history[-1]["role"] == "user": self.history.pop()
                return {"status": "error", "content": "Модель не сгенерировала ответ. Попробуйте еще раз."}

            if TOOL_CALL_MARKER in model_response_text:
                parts = model_response_text.split(TOOL_CALL_MARKER, 1)
                user_message = parts[0].strip()
                json_str = parts[1].strip()
                
                try:
                    tool_call_data = json.loads(json_str)
                    tool_name = tool_call_data.get("tool")
                    if tool_name not in self.tools:
                        raise ValueError(f"Модель вызвала неизвестный инструмент: {tool_name}")

                    self.log(f"[TOOL] Модель решила использовать '{tool_name}'. Сообщение: '{user_message}'. Запрос: '{tool_call_data.get('query')}'")
                    return {
                        "status": "tool_call",
                        "user_message": user_message,
                        "tool_data": tool_call_data,
                        "full_model_response": model_response_text
                    }
                except (json.JSONDecodeError, ValueError) as e:
                    self.log(f"[ERROR] Ошибка в вызове инструмента: {e}")
                    self.history.append({"role": "assistant", "content": model_response_text})
                    return {"status": "done", "content": model_response_text}
            else:
                self.history.append({"role": "assistant", "content": model_response_text})
                return {"status": "done", "content": model_response_text}
                
        except Exception:
            self.log(f"[ERROR] Ошибка генерации ответа!\n{traceback.format_exc()}")
            if self.history and self.history[-1]["role"] == "user": self.history.pop()
            return {"status": "error", "content": "Произошла критическая ошибка при генерации ответа."}

    def execute_tool_and_continue(self, tool_call_data, full_model_response):
        tool_name = tool_call_data.get("tool")
        query = tool_call_data.get("query")
        tool_url = self.tools[tool_name]["url"]

        try:
            self.log(f"[MCP Client] Отправка запроса к '{tool_name}' на {tool_url}...")
            # Стандартная структура MCP
            payload = {"action": {"type": "browse", "goal": query}} # В будущем можно будет кастомизировать 'type'
            response = requests.post(tool_url, json=payload, timeout=120)
            response.raise_for_status()
            
            result_content = response.json().get("result", "Инструмент не вернул результат.")
            self.log(f"[MCP Client] Успех! Получен результат от '{tool_name}'.")
            
            self.history.append({"role": "assistant", "content": full_model_response})
            self.history.append({"role": "tool_result", "content": result_content})
            
            final_prompt = self._create_chat_prompt()
            final_output = self.llm(prompt=final_prompt, max_tokens=512, stop=["<|im_end|>"])
            final_result = final_output['choices'][0]['text'].strip()
            
            self.history.append({"role": "assistant", "content": final_result})
            return {"status": "done", "content": final_result}

        except requests.exceptions.RequestException as e:
            error_message = f"Ошибка при вызове инструмента '{tool_name}': {e}"
            self.log(f"[ERROR] {error_message}")
            if self.history and self.history[-1]["role"] == "user": self.history.pop()
            # Возвращаем ошибку модели, чтобы она могла сказать об этом пользователю
            self.history.append({"role": "assistant", "content": full_model_response})
            self.history.append({"role": "tool_result", "content": f"Не удалось выполнить запрос: {error_message}"})
            final_prompt = self._create_chat_prompt()
            final_output = self.llm(prompt=final_prompt, max_tokens=512, stop=["<|im_end|>"])
            final_result = final_output['choices'][0]['text'].strip()
            self.history.append({"role": "assistant", "content": final_result})
            return {"status": "done", "content": final_result}
