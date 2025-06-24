# engine.py - КОД ОСТАЕТСЯ БЕЗ ИЗМЕНЕНИЙ, ТАК КАК ЗАЩИТА УЖЕ БЫЛА ДОБАВЛЕНА

import os
import traceback
import json
import threading
import time
from queue import Queue
from llama_cpp import Llama
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
        self.model_configs = {
            "default": "cognitivecomputations_Dolphin-Mistral-24B-Venice-Edition-Q4_K_M.gguf",
            "coding": "Devstral-Small-2505-IQ4_NL.gguf"
        }
        
        self.tools_config = {}
        self.settings = {}
        self.assistant_name = "Ассистент"
        self.task_queue = Queue()
        
        self.current_llm = None
        self.current_model_key = None
        self.model_lock = threading.Lock()

        self.log_callback("[Engine] Движок инициализирован.")
        self.reload_settings()
        self._load_tools_config()
        
        self.processing_thread = threading.Thread(target=self._processing_loop, daemon=True)
        self.processing_thread.start()
        
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
        self.log_callback(f"[Engine] Имя ассистента установлено: '{self.assistant_name}'")

        if self.voice_controller:
            self.voice_controller.reload()

    def set_voice_controller(self, vc):
        self.voice_controller = vc
        self.log_callback("[Engine] Голосовой контроллер успешно подключен.")

    def toggle_voice_engine(self, state: bool):
        if self.voice_controller:
            if state: self.voice_controller.start_listening()
            else: self.voice_controller.stop_listening()

    def _load_tools_config(self):
        try:
            with open("tools_config.json", "r", encoding="utf-8") as f: self.tools_config = json.load(f)
            self.log_callback("[Engine] Конфигурация инструментов успешно загружена.")
        except (FileNotFoundError, json.JSONDecodeError) as e:
            self.log_callback(f"[Engine] [CRITICAL ERROR] Не удалось загрузить tools_config.json: {e}"); self.tools_config = {}

    def _switch_model(self, model_key: str):
        with self.model_lock:
            if model_key == self.current_model_key: return True
            if self.current_llm is not None:
                self.log_callback(f"[Engine] Выгружаю модель '{self.current_model_key}'...")
                if self.busy_callback: self.busy_callback(True, f"Выгружаю модель '{self.current_model_key}'...")
                self.current_llm = None; self.current_model_key = None
            model_name = self.model_configs.get(model_key)
            if not model_name:
                self.log_callback(f"[Engine] [CRITICAL ERROR] Конфигурация для модели '{model_key}' не найдена.")
                return False
            model_path = os.path.join(self.MODELS_DIR, model_name)
            if not os.path.exists(model_path):
                self.log_callback(f"[Engine] [CRITICAL ERROR] Файл модели не найден: {model_path}")
                return False
            try:
                self.log_callback(f"[Engine] Загружаю модель '{model_key}' ({model_name})...")
                if self.busy_callback: self.busy_callback(True, f"Загружаю модель '{model_key}'...")
                self.current_llm = Llama(model_path=model_path, n_gpu_layers=-1, n_ctx=4096, verbose=False)
                self.current_model_key = model_key
                self.log_callback(f"[Engine] Модель '{model_key}' успешно загружена!")
                return True
            except Exception:
                self.log_callback(f"[Engine] [CRITICAL ERROR] ОШИБКА ЗАГРУЗКИ МОДЕЛИ '{model_key}'!\n{traceback.format_exc()}")
                self.current_llm = None; self.current_model_key = None
                return False

    def initial_load(self):
        if not self._switch_model("default"):
             if self.busy_callback: self.busy_callback(False, "КРИТИЧЕСКАЯ ОШИБКА ЗАГРУЗКИ МОДЕЛИ!")
        else:
             if self.busy_callback: self.busy_callback(False, "Готов к работе. Ожидание задач...")
        
    def submit_task(self, prompt: str):
        self.task_queue.put(prompt)

    def _processing_loop(self):
        while self.current_llm is None: time.sleep(0.5)
        self.log_callback("[Processing Loop] Модель готова. Начинаю обработку очереди.")
        while True:
            prompt = self.task_queue.get()
            if self.busy_callback: self.busy_callback(True, f"Обработка: {prompt[:40]}...")
            if self.update_callback: self.update_callback({"type": "user_prompt", "data": prompt})
            self._process_single_task(prompt)
            if self.busy_callback: self.busy_callback(False, "Готов к работе. Ожидание задач...")
            self.task_queue.task_done()

    def _process_single_task(self, user_prompt: str):
        final_answer = ""
        try:
            llm_start_time = time.time()
            self.log_callback("[Engine] >>> НАЧАЛО РАБОТЫ LLM/КОМАНДЫ...")
            
            if not self._switch_model("default"): return
            dispatcher = DispatcherAgent(self.current_llm, self.log_callback)
            self.log_callback(f"[Dispatcher] Получена задача: '{user_prompt}'. Определяю тип и модель...")
            task_routing = dispatcher.choose_crew_and_model(user_prompt)
            crew_type = task_routing['crew_type']; model_key = task_routing['model_key']
            self.log_callback(f"[Dispatcher] Задача классифицирована как '{crew_type}'. Рекомендованная модель: '{model_key}'.")
            if not self._switch_model(model_key): raise Exception(f"Не удалось переключиться на модель {model_key}")
            
            crew_map = {"coding": CodingCrew, "research": ResearchCrew, "browsing": BrowserCrew, "documentation_query": DocsCrew}
            crew_class = crew_map.get(crew_type)
            
            if crew_class:
                crew_to_run = crew_class(self.current_llm, self.tools_config)
                result = crew_to_run.run(user_prompt, self.log_callback, self.update_callback)
                
                if result:
                    final_answer = result.get("final_result", "Команда завершила работу без финального ответа.")
                else:
                    error_msg = f"Команда '{crew_type}' завершилась некорректно и не вернула результат (None)."
                    self.log_callback(f"[Engine] [ERROR] {error_msg}")
                    final_answer = "Произошла внутренняя ошибка: команда не вернула результат."
                    if self.update_callback:
                        self.update_callback({"type": "error", "data": error_msg})

            else: 
                self.log_callback("[Dispatcher] Простое приветствие. Отвечаю напрямую...")
                system_prompt = f"Ты — полезный ассистент по имени {self.assistant_name}."
                prompt = f"<|im_start|>system\n{system_prompt}<|im_end|>\n<|im_start|>user\n{user_prompt}<|im_end|>\n<|im_start|>assistant\n"
                output = self.current_llm(prompt, max_tokens=512, stop=["<|im_end|>"]); answer = output['choices'][0]['text'].strip()
                final_answer = answer
                if self.update_callback:
                    result_data = {"final_result": answer, "trajectory": ["Ответ сгенерирован напрямую, без использования команд."]}
                    self.update_callback({"type": "final_result", "data": result_data})
            
            llm_end_time = time.time()
            self.log_callback(f"[Engine] <<< КОНЕЦ РАБОТЫ LLM/КОМАНДЫ. ВРЕМЯ ВЫПОЛНЕНИЯ: {llm_end_time - llm_start_time:.2f} сек.")

        except Exception as e:
            error_message = f"Критическая ошибка в работе команды: {traceback.format_exc()}"
            self.log_callback(f"[Engine] [ERROR] {error_message}")
            final_answer = f"Произошла критическая ошибка: {e}"
            if self.update_callback: self.update_callback({"type": "error", "data": f"Критическая ошибка в работе AI-агентов: {e}"})
        
        if self.voice_controller and final_answer:
            self.log_callback("[Engine] Отправка финального ответа в TTS...")
            self.voice_controller.say(final_answer)