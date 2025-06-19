# engine.py - Движок, который умеет вести диалог

import os
import gc
import traceback
from llama_cpp import Llama

class OrchestratorEngine:
    def __init__(self, log_callback):
        self.log = log_callback
        self.MODELS_DIR = r"D:\nn\models"
        self.llm = None
        self.model_name = None
        self.history = [] # <-- ГЛАВНОЕ: Добавляем историю чата
        self.log("[Engine] Движок v2 инициализирован.")

    def get_available_models(self):
        try:
            return [f for f in os.listdir(self.MODELS_DIR) if f.endswith('.gguf')]
        except FileNotFoundError:
            self.log(f"[ERROR] Папка с моделями не найдена: {self.MODELS_DIR}")
            return []

    def _create_chat_prompt(self):
        """Создает полный промпт для модели из истории диалога."""
        # Это стандартный формат ChatML, который понимают многие модели, включая Dolphin.
        prompt_str = ""
        for message in self.history:
            prompt_str += f"<|im_start|>{message['role']}\n{message['content']}<|im_end|>\n"
        prompt_str += "<|im_start|>assistant\n"
        self.log(f"[DEBUG] Сгенерирован промпт:\n{prompt_str}")
        return prompt_str

    def load_model(self, model_name_to_load):
        if not model_name_to_load:
            self.log("[ERROR] Имя модели для загрузки не указано.")
            return False # Возвращаем результат операции
        
        self.unload_model() # Всегда выгружаем старую модель
        
        model_path = os.path.join(self.MODELS_DIR, model_name_to_load)
        self.log(f"[INFO] Начинаю загрузку модели: {model_name_to_load}")
        try:
            loaded_llm = Llama(model_path=model_path, n_gpu_layers=-1, n_ctx=4096, verbose=False)
            self.llm = loaded_llm
            self.model_name = model_name_to_load
            self.log(f"[SUCCESS] ✅ Модель '{self.model_name}' успешно загружена на GPU!")
            return True
        except Exception:
            error_info = traceback.format_exc()
            self.log(f"[ERROR] ❌ ПРОВАЛ ЗАГРУЗКИ МОДЕЛИ!\n{error_info}")
            self.llm = None
            self.model_name = None
            return False

    def unload_model(self):
        if self.llm is None: return
        self.log(f"[INFO] Выгружаю модель '{self.model_name}'...")
        self.llm = None
        self.model_name = None
        self.history = [] # Очищаем историю при выгрузке
        gc.collect()
        self.log("[INFO] Модель выгружена, память освобождена.")

    def get_response(self, user_prompt):
        if self.llm is None:
            return "Модель не загружена."

        # Добавляем сообщение пользователя в историю
        self.history.append({"role": "user", "content": user_prompt})
        
        # Создаем полный, правильно отформатированный промпт
        full_prompt = self._create_chat_prompt()
        
        try:
            output = self.llm(prompt=full_prompt, max_tokens=512, stop=["<|im_end|>"])
            result = output['choices'][0]['text'].strip()
            
            # Добавляем ответ модели в историю
            self.history.append({"role": "assistant", "content": result})
            self.log(f"[INFO] Ответ модели: '{result}'")
            return result
        except Exception:
            error_info = traceback.format_exc()
            self.log(f"[ERROR] ❌ Ошибка во время генерации ответа!\n{error_info}")
            return "Произошла ошибка при генерации ответа."