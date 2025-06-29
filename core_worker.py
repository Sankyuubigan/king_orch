import threading
import logging
import os
import json
import gc
from queue import Queue, Empty
from typing import List

from langchain_community.chat_models import ChatLlamaCpp
from graphs.coding_graph import create_coding_graph
from graphs.research_graph import create_research_graph
from graphs.dispatcher_graph import create_dispatcher_graph
from graphs.browser_graph import create_browser_graph
from mcp_use import MCPClient
from ui_adapter import UiAdapter

MODELS_DIR = r"D:\nn\models"
SETTINGS_FILE = "settings.json"
logger = logging.getLogger(__name__)

class CoreWorker:
    """
    Управляет жизненным циклом LLM. Полностью отделен от UI.
    """
    def __init__(self, task_queue: Queue, mcp_client: MCPClient, listener: UiAdapter):
        self.task_queue = task_queue
        self.mcp_client = mcp_client
        self.listener = listener # ИЗМЕНЕНИЕ: Храним ссылку на адаптер, а не на UI
        self.reload_event = threading.Event()
        self.stop_event = threading.Event()

        self.llm = None
        self.dispatcher = None

    def run(self):
        """Основной цикл работы воркера."""
        self._initialize_components()

        while not self.stop_event.is_set():
            if self.reload_event.is_set():
                self._perform_reload()
                continue

            try:
                task = self.task_queue.get(timeout=1.0)
                if task is None:
                    break
                
                self._process_task(task)
                self.task_queue.task_done()

            except Empty:
                continue

    def _process_task(self, task):
        """Обработка одной задачи из очереди."""
        if not self.dispatcher:
            logger.error("[CoreWorker] Диспетчер не инициализирован, задача не может быть обработана.")
            self.listener.on_final_result("Ошибка: Основные компоненты не загружены.")
            return

        try:
            self.listener.on_busy_changed(True, f"Обработка: {task[:50]}...")
            self.listener.on_log_message(f"[Queue] Получена задача: {task}")

            final_state = self.dispatcher.invoke({"task": task})
            final_result = final_state.get("result", "Задача выполнена, но финальный результат отсутствует.")
            
            self.listener.on_log_message(f"[Dispatcher] Финальный результат: {final_result}")
            self.listener.on_final_result(final_result)
        except Exception as e:
            logger.critical(f"Ошибка при обработке задачи '{task}': {e}", exc_info=True)
            self.listener.on_final_result(f"Критическая ошибка при обработке задачи: {e}")
        finally:
            self.listener.on_busy_changed(False)

    def _initialize_components(self):
        """Загружает LLM и пересобирает графы."""
        self.listener.on_busy_changed(True, "Загрузка LLM и графов...")
        try:
            settings = self._load_settings()
            model_filename = settings.get("llm_model_file")
            
            if model_filename and os.path.exists(os.path.join(MODELS_DIR, model_filename)):
                model_path = os.path.join(MODELS_DIR, model_filename)
                logger.info(f"[CoreWorker] Загрузка модели из настроек: {model_filename}")
            else:
                model_path = os.path.join(MODELS_DIR, "cognitivecomputations_Dolphin-Mistral-24B-Venice-Edition-Q4_K_M.gguf")
                logger.warning(f"[CoreWorker] Модель '{model_filename}' не найдена или не указана. Используется модель по умолчанию.")

            if not os.path.exists(model_path):
                raise FileNotFoundError(f"Файл основной модели не найден: {model_path}")

            self.llm = ChatLlamaCpp(
                model_path=model_path,
                temperature=0.1,
                n_gpu_layers=-1,
                n_ctx=4096,
                verbose=False
            )
            logger.info("[CoreWorker] Основная LLM модель успешно загружена.")

            logger.info("[CoreWorker] Сборка графов LangGraph...")
            coding_graph = create_coding_graph(self.llm, self.mcp_client)
            research_graph = create_research_graph(self.llm, self.mcp_client)
            browser_graph = create_browser_graph(self.llm, self.mcp_client)
            self.dispatcher = create_dispatcher_graph(self.llm, coding_graph, research_graph, browser_graph)
            logger.info("[CoreWorker] Все графы успешно собраны.")
            self.listener.on_busy_changed(False)

        except Exception as e:
            logger.critical(f"[CoreWorker] Критическая ошибка при инициализации: {e}", exc_info=True)
            self.listener.on_critical_error(f"ОШИБКА: {e}")

    def _shutdown_components(self):
        """Выгружает компоненты и очищает память."""
        self.listener.on_log_message("[CoreWorker] Выгрузка LLM и графов...")
        del self.llm
        del self.dispatcher
        self.llm = None
        self.dispatcher = None
        gc.collect()
        self.listener.on_log_message("[CoreWorker] Компоненты выгружены, память освобождена.")

    def _perform_reload(self):
        """Полный цикл перезагрузки."""
        self.listener.on_busy_changed(True, "Перезагрузка LLM...")
        self._shutdown_components()
        self._initialize_components()
        self.reload_event.clear()

    def trigger_reload(self):
        """Сигнал для начала перезагрузки из внешнего потока."""
        logger.info("[CoreWorker] Получен сигнал на 'горячую' перезагрузку.")
        self.reload_event.set()

    def trigger_stop(self):
        """Сигнал для полной остановки воркера."""
        self.stop_event.set()
        self.task_queue.put(None)

    def _load_settings(self):
        if os.path.exists(SETTINGS_FILE):
            try:
                with open(SETTINGS_FILE, "r", encoding="utf-8") as f:
                    return json.load(f)
            except json.JSONDecodeError:
                return {}
        return {}