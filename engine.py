# engine.py - ЦЕНТРАЛИЗОВАНА ЗАГРУЗКА КОНФИГУРАЦИИ

import os
import traceback
import json
from llama_cpp import Llama
from crews.research_crew import ResearchCrew
from crews.coding_crew import CodingCrew
from agents.dispatcher_agent import DispatcherAgent

class OrchestratorEngine:
    def __init__(self, log_callback):
        self.log = log_callback
        self.MODELS_DIR = r"D:\nn\models"
        self.model_name = "cognitivecomputations_Dolphin-Mistral-24B-Venice-Edition-Q4_K_M.gguf"
        self.model_path = os.path.join(self.MODELS_DIR, self.model_name)
        self.llm = None
        self.tools_config = {}
        self.log("[Engine] Движок инициализирован.")
        self._load_tools_config()

    def _load_tools_config(self):
        """Загружает конфигурацию инструментов один раз при запуске."""
        try:
            with open("tools_config.json", "r", encoding="utf-8") as f:
                self.tools_config = json.load(f)
            self.log("[Engine] Конфигурация инструментов успешно загружена.")
        except (FileNotFoundError, json.JSONDecodeError) as e:
            self.log(f"[Engine] [CRITICAL ERROR] Не удалось загрузить tools_config.json: {e}")
            self.tools_config = {} # Используем пустой конфиг, чтобы не падать

    def load_model(self):
        if self.llm:
            self.log("[Engine] Модель уже загружена.")
            return True
        if not os.path.exists(self.model_path):
            self.log(f"[Engine] [ERROR] Файл модели не найден: {self.model_path}")
            return False
        try:
            self.log(f"[Engine] Загрузка модели: {self.model_name}...")
            self.llm = Llama(model_path=self.model_path, n_gpu_layers=-1, n_ctx=4096, verbose=False)
            self.log("[Engine] Модель успешно загружена!")
            return True
        except Exception:
            self.log(f"[Engine] [ERROR] КРИТИЧЕСКАЯ ОШИБКА ЗАГРУЗКИ МОДЕЛИ!\n{traceback.format_exc()}")
            self.llm = None
            return False

    def get_response(self, user_prompt):
        if not self.llm:
            return {"status": "error", "content": "Модель не загружена."}
            
        try:
            self.log(f"[Dispatcher] Получена задача: '{user_prompt}'. Определяю тип задачи...")
            dispatcher = DispatcherAgent(self.llm, self.log)
            crew_type = dispatcher.choose_crew(user_prompt)
            self.log(f"[Dispatcher] Задача классифицирована как '{crew_type}'.")

            crew_to_run = None
            if crew_type == "coding":
                self.log("[Dispatcher] Нанимаю команду 'Кодеры'...")
                crew_to_run = CodingCrew(self.llm, self.tools_config) # Передаем конфиг
            elif crew_type == "research":
                self.log("[Dispatcher] Нанимаю команду 'Исследователи'...")
                crew_to_run = ResearchCrew(self.llm, self.tools_config) # Передаем конфиг
            else: 
                self.log("[Dispatcher] Простое приветствие или диалог. Отвечаю напрямую...")

            if crew_to_run:
                result_data = crew_to_run.run(user_prompt, self.log)
            else: 
                prompt = f"<|im_start|>system\nТы — полезный ассистент.<|im_end|>\n<|im_start|>user\n{user_prompt}<|im_end|>\n<|im_start|>assistant\n"
                output = self.llm(prompt, max_tokens=512, stop=["<|im_end|>"])
                answer = output['choices'][0]['text'].strip()
                result_data = {
                    "final_result": answer,
                    "trajectory": ["Ответ сгенерирован напрямую, без использования команд."]
                }
            
            self.log("[Dispatcher] Задача обработана.")
            return {"status": "done", "content": result_data}

        except Exception:
            error_message = f"Критическая ошибка в работе команды: {traceback.format_exc()}"
            self.log(f"[Dispatcher] [ERROR] {error_message}")
            return {"status": "error", "content": "Критическая ошибка в работе AI-агентов."}