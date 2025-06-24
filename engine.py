# engine.py - МОДЕРНИЗИРОВАН В МУЛЬТИ-МОДЕЛЬНЫЙ ЦЕНТР С ПОДДЕРЖКОЙ OCR

import os
import traceback
import json
import threading
import time
from queue import Queue
from llama_cpp import Llama
import base64 # Для работы с OCR

from crews.research_crew import ResearchCrew
from crews.coding_crew import CodingCrew
from crews.browser_crew import BrowserCrew
from crews.docs_crew import DocsCrew
from agents.dispatcher_agent import DispatcherAgent

SETTINGS_FILE = "settings.json"

class OrchestratorEngine:
    def __init__(self, log_callback):
        self.log_callback = log_callback
        self.busy_callback = None
        self.update_callback = None
        self.voice_controller = None

        self.MODELS_DIR = r"D:\nn\models"
        # ИЗМЕНЕНО: Добавлена OCR-модель
        self.model_configs = {
            "default": "cognitivecomputations_Dolphin-Mistral-24B-Venice-Edition-Q4_K_M.gguf",
            "coding": "Devstral-Small-2505-IQ4_NL.gguf",
            "ocr": "Nanonets-OCR-s-IQ4_NL.gguf"
        }
        
        self.tools_config = {}
        self.settings = {}
        self.assistant_name = "Ассистент"
        self.task_queue = Queue()
        
        # ИЗМЕНЕНО: Движок теперь управляет словарем загруженных моделей
        self.loaded_models = {}
        self.model_lock = threading.Lock() # Защищает доступ к словарю моделей

        self.log_callback("[Engine] Движок инициализирован.")
        self.reload_settings()
        self._load_tools_config()
        
        self.processing_thread = threading.Thread(target=self._processing_loop, daemon=True)
        self.processing_thread.start()

    def _get_model(self, model_key: str) -> Llama | None:
        """Загружает модель по требованию и кэширует ее."""
        with self.model_lock:
            if model_key in self.loaded_models:
                return self.loaded_models[model_key]
            
            model_name = self.model_configs.get(model_key)
            if not model_name:
                self.log_callback(f"[Engine] [ERROR] Конфигурация для модели '{model_key}' не найдена.")
                return None
            
            model_path = os.path.join(self.MODELS_DIR, model_name)
            if not os.path.exists(model_path):
                self.log_callback(f"[Engine] [ERROR] Файл модели не найден: {model_path}")
                return None
            
            try:
                self.log_callback(f"[Engine] Загружаю модель '{model_key}' ({model_name})...")
                if self.busy_callback: self.busy_callback(True, f"Загружаю модель '{model_key}'...")

                # OCR модели требуют специальной конфигурации
                if model_key == "ocr":
                    llm = Llama(model_path=model_path, n_gpu_layers=1, n_ctx=1024, logits_all=True, verbose=False)
                else:
                    llm = Llama(model_path=model_path, n_gpu_layers=-1, n_ctx=4096, verbose=False)
                
                self.loaded_models[model_key] = llm
                self.log_callback(f"[Engine] Модель '{model_key}' успешно загружена!")
                if self.busy_callback: self.busy_callback(False)
                return llm
            except Exception as e:
                self.log_callback(f"[Engine] [CRITICAL ERROR] ОШИБКА ЗАГРУЗКИ МОДЕЛИ '{model_key}'!\n{traceback.format_exc()}")
                if self.busy_callback: self.busy_callback(False)
                return None

    def route_internal_call(self, tool_name: str, tool_params: dict):
        """Маршрутизатор для вызова внутренних инструментов движка."""
        if tool_name == "local_ocr":
            return self._execute_ocr(tool_params.get("image_b64"))
        return {"error": f"Неизвестный внутренний инструмент: {tool_name}"}

    def _execute_ocr(self, image_b64: str) -> dict:
        """Выполняет распознавание текста на изображении."""
        self.log_callback("[Engine-OCR] Получен запрос на распознавание.")
        if not image_b64:
            return {"error": "Изображение для OCR не предоставлено."}
            
        ocr_model = self._get_model("ocr")
        if not ocr_model:
            return {"error": "OCR модель не загружена."}
            
        try:
            # LLaVA модели ожидают prompt в определенном формате
            prompt = "Пожалуйста, предоставь детальное описание этого изображения."
            
            # В llama-cpp-python изображения передаются как часть messages
            result = ocr_model.create_chat_completion(
                messages=[
                    {
                        "role": "user",
                        "content": [
                            {"type": "text", "text": prompt},
                            {"type": "image_url", "image_url": {"url": f"data:image/jpeg;base64,{image_b64}"}}
                        ]
                    }
                ]
            )
            # Извлекаем распознанный текст
            content = result['choices'][0]['message']['content']
            # В реальном OCR-парсере здесь была бы логика извлечения координат (box)
            # Для Nanonets-OCR это может потребовать анализа logits
            # Сейчас мы просто возвращаем весь текст для отладки
            return {"text": content, "boxes": []} 
        except Exception as e:
            self.log_callback(f"[Engine-OCR] [ERROR] Ошибка при распознавании: {e}")
            return {"error": f"Ошибка OCR: {e}"}

    def initial_load(self):
        if not self._get_model("default"):
             if self.busy_callback: self.busy_callback(False, "КРИТИЧЕСКАЯ ОШИБКА ЗАГРУЗКИ МОДЕЛИ!")
        else:
             if self.busy_callback: self.busy_callback(False, "Готов к работе. Ожидание задач...")
        
    def _process_single_task(self, user_prompt: str):
        final_answer = ""
        try:
            dispatcher_llm = self._get_model("default")
            if not dispatcher_llm: return
            
            dispatcher = DispatcherAgent(dispatcher_llm, self.log_callback)
            task_routing = dispatcher.choose_crew_and_model(user_prompt)
            crew_type = task_routing['crew_type']; model_key = task_routing['model_key']
            
            task_llm = self._get_model(model_key)
            if not task_llm: return

            crew_map = {"coding": CodingCrew, "research": ResearchCrew, "browsing": BrowserCrew, "documentation_query": DocsCrew}
            crew_class = crew_map.get(crew_type)
            
            if crew_class:
                # ПЕРЕДАЕМ ДВИЖОК В КОМАНДУ, чтобы они могли вызывать внутренние инструменты
                crew_to_run = crew_class(task_llm, self.tools_config, self) 
                result = crew_to_run.run(user_prompt, self.log_callback, self.update_callback)
                if result: final_answer = result.get("final_result", "[Команда не вернула ответ]")
            else: 
                # Прямой ответ
                output = task_llm(f"<|im_start|>user\n{user_prompt}<|im_end|>\n<|im_start|>assistant\n", max_tokens=512, stop=["<|im_end|>"])
                final_answer = output['choices'][0]['text'].strip()
                if self.update_callback: self.update_callback({"type": "final_result", "data": {"final_result": final_answer}})
        
        except Exception as e:
            error_message = f"Критическая ошибка в работе команды: {traceback.format_exc()}"
            self.log_callback(f"[Engine] [ERROR] {error_message}")
            final_answer = f"Произошла критическая ошибка: {e}"
            if self.update_callback: self.update_callback({"type": "error", "data": error_message})
        
        if self.voice_controller and final_answer: self.voice_controller.say(final_answer)

    # Остальной код без изменений
    def reload_settings(self):
        self.log_callback("[Engine] Загрузка/перезагрузка настроек...")
        try:
            if os.path.exists(SETTINGS_FILE):
                with open(SETTINGS_FILE, "r", encoding="utf-8") as f:
                    self.settings = json.load(f)
            else:
                self.settings = {}
        except Exception as e:
            self.log_callback(f"[Engine] [ERROR] Не удалось прочитать {SETTINGS_FILE}: {e}")
            self.settings = {}
        self.assistant_name = self.settings.get("assistant_name", "Ассистент")
        if self.voice_controller: self.voice_controller.reload()
    def set_voice_controller(self, vc):
        self.voice_controller = vc
    def toggle_voice_engine(self, state: bool):
        if self.voice_controller:
            if state: self.voice_controller.start_listening()
            else: self.voice_controller.stop_listening()
    def _load_tools_config(self):
        try:
            with open("tools_config.json", "r", encoding="utf-8") as f: self.tools_config = json.load(f)
        except Exception as e: self.tools_config = {}
    def submit_task(self, prompt: str):
        self.task_queue.put(prompt)
    def _processing_loop(self):
        while not self.loaded_models.get("default"): time.sleep(0.5)
        while True:
            prompt = self.task_queue.get()
            if self.busy_callback: self.busy_callback(True, f"Обработка: {prompt[:40]}...")
            if self.update_callback: self.update_callback({"type": "user_prompt", "data": prompt})
            self._process_single_task(prompt)
            if self.busy_callback: self.busy_callback(False, "Готов к работе.")
            self.task_queue.task_done()