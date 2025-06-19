# engine.py - ИСПРАВЛЕННАЯ ВЕРСИЯ

import os
import gc
import json
import traceback
import requests
from llama_cpp import Llama

MCP_SERVER_URL = "http://localhost:7777/v1/action"

class OrchestratorEngine:
    def __init__(self, log_callback):
        self.log = log_callback
        self.MODELS_DIR = r"D:\nn\models"
        self.llm = None
        self.model_name = None
        self.history = []
        
        try:
            with open("system_prompt.md", "r", encoding="utf-8") as f:
                self.system_prompt = f.read()
            self.log("[Engine] Системный промпт загружен.")
        except FileNotFoundError:
            self.log("[ERROR] Файл system_prompt.md не найден!")
            self.system_prompt = "Ты — полезный ассистент."
        
        self.log("[Engine] Движок инициализирован.")

    def get_available_models(self):
        try:
            return [f for f in os.listdir(self.MODELS_DIR) if f.endswith('.gguf')]
        except FileNotFoundError:
            self.log(f"[ERROR] Папка с моделями не найдена: {self.MODELS_DIR}")
            return []

    def _create_chat_prompt(self):
        system_message = {"role": "system", "content": self.system_prompt}
        full_history = [system_message] + self.history
        prompt_str = ""
        for message in full_history:
            if message.get("role") == "tool_result":
                 prompt_str += f"Результат от инструмента stagehand_search:\n{message['content']}\n"
            else:
                 prompt_str += f"<|im_start|>{message['role']}\n{message['content']}<|im_end|>\n"
        prompt_str += "<|im_start|>assistant\n"
        return prompt_str

    def get_current_token_count(self):
        if not self.llm or not self.history:
            return 0
        full_prompt_for_counting = self._create_chat_prompt()
        tokens = self.llm.tokenize(full_prompt_for_counting.encode("utf-8"))
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
        if self.llm is None: return "Модель не загружена."
        self.history.append({"role": "user", "content": user_prompt})
        full_prompt = self._create_chat_prompt()
        
        try:
            # Вызов модели для получения ответа
            output = self.llm(prompt=full_prompt, max_tokens=512, stop=["<|im_end|>", "```"])
            
            ####################################################################
            ### ИСПРАВЛЕНИЕ ЗДЕСЬ (1/2): ДОБАВЛЕН ИНДЕКС               ###
            ####################################################################
            # 'choices' - это список, поэтому получаем первый элемент
            model_response_text = output['choices'][0]['text'].strip()
            
            try:
                # Попытка найти и обработать вызов инструмента
                json_part = model_response_text[model_response_text.find('{'):model_response_text.rfind('}')+1]
                tool_call = json.loads(json_part)

                if tool_call.get("tool") == "stagehand_search":
                    query = tool_call.get("query")
                    self.log(f"[TOOL] Модель запросила поиск: '{query}'")
                    
                    try:
                        self.log(f"[MCP Client] Отправка запроса на {MCP_SERVER_URL}...")
                        payload = {"action": {"type": "browse", "goal": query}}
                        response = requests.post(MCP_SERVER_URL, json=payload, timeout=120)
                        response.raise_for_status()
                        result_content = response.json().get("result", "Инструмент не вернул результат.")
                        self.log(f"[MCP Client] Получен результат.")
                    except requests.exceptions.RequestException as e:
                        self.log(f"[ERROR] Ошибка подключения к MCP-серверу: {e}")
                        result_content = "Не удалось подключиться к инструменту поиска. Убедитесь, что mcp_server.js запущен."

                    self.history.append({"role": "assistant", "content": model_response_text})
                    self.history.append({"role": "tool_result", "content": result_content})
                    
                    # Второй вызов модели с результатами от инструмента
                    final_prompt = self._create_chat_prompt()
                    final_output = self.llm(prompt=final_prompt, max_tokens=512, stop=["<|im_end|>"])
                    
                    ####################################################################
                    ### ИСПРАВЛЕНИЕ ЗДЕСЬ (2/2): ДОБАВЛЕН ИНДЕКС               ###
                    ####################################################################
                    final_result = final_output['choices'][0]['text'].strip()
                    
                    self.history.append({"role": "assistant", "content": final_result})
                    return final_result
            except (json.JSONDecodeError, KeyError):
                # Если это не вызов инструмента, просто возвращаем ответ
                self.history.append({"role": "assistant", "content": model_response_text})
                return model_response_text
        except Exception:
            self.log(f"[ERROR] Ошибка генерации ответа!\n{traceback.format_exc()}")
            # Важно очистить последний user_prompt из истории, если произошла ошибка
            if self.history and self.history[-1]["role"] == "user":
                self.history.pop()
            return "Произошла ошибка при генерации ответа."