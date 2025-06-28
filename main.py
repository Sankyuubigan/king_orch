import tkinter as tk
import os
import threading
from queue import Queue
import logging
import atexit
import asyncio
import json

# --- 1. Настройка окружения и логирования ---
os.environ['MCP_USE_ANONYMIZED_TELEMETRY'] = 'false'
from utils.logging_utils import setup_logging
setup_logging()

# --- 2. Импорт основных компонентов ---
from langchain_community.chat_models import ChatLlamaCpp
from mcp_use import MCPClient
from ui import AppUI
from graphs.coding_graph import create_coding_graph
from graphs.research_graph import create_research_graph
from graphs.dispatcher_graph import create_dispatcher_graph
from graphs.browser_graph import create_browser_graph
from voice_engine.controller import VoiceController
# ИСПРАВЛЕНИЕ: Возвращаемся к использованию функции-фиксера
from utils.prompt_templates import get_fixed_chat_handler

# --- 3. Глобальные переменные ---
task_queue = Queue()
app_ui = None
mcp_client = None
voice_controller = None
is_shutting_down = False
MODELS_DIR = r"D:\nn\models"
SETTINGS_FILE = "settings.json"
logger = logging.getLogger(__name__)

def create_dirs():
    dirs = [
        "graphs", "prompts", "mcp_servers", "utils", "tools",
        "voice_engine/stt", "voice_engine/tts", "voice_engine/silero_cache",
        "vendor", "workspace", "logs"
    ]
    for d in dirs:
        os.makedirs(d, exist_ok=True)
    
    for d in ["graphs", "utils", "voice_engine", "mcp_servers", "prompts"]:
        init_file = os.path.join(d, "__init__.py")
        if not os.path.exists(init_file):
            with open(init_file, "a"): pass

def load_settings():
    if os.path.exists(SETTINGS_FILE):
        try:
            with open(SETTINGS_FILE, "r", encoding="utf-8") as f:
                return json.load(f)
        except json.JSONDecodeError:
            logger.warning("Ошибка чтения settings.json. Файл поврежден. Будут использованы значения по умолчанию.")
            return {}
    return {}

def background_processing_loop():
    global mcp_client, app_ui, voice_controller
    
    try:
        if app_ui: app_ui.set_info_label("Загрузка настроек и основной LLM...")
        
        settings = load_settings()
        model_filename = settings.get("llm_model_file")
        
        if model_filename and os.path.exists(os.path.join(MODELS_DIR, model_filename)):
            model_path = os.path.join(MODELS_DIR, model_filename)
            logger.info(f"Загрузка модели из настроек: {model_filename}")
        else:
            model_path = os.path.join(MODELS_DIR, "cognitivecomputations_Dolphin-Mistral-24B-Venice-Edition-Q4_K_M.gguf")
            if model_filename:
                logger.warning(f"Модель '{model_filename}' не найдена. Используется модель по умолчанию.")
            else:
                logger.info("Модель в настройках не указана. Используется модель по умолчанию.")

        if not os.path.exists(model_path):
            critical_error = f"Файл основной модели не найден: {model_path}"
            logger.critical(critical_error)
            if app_ui: app_ui.set_info_label(f"ОШИБКА: {critical_error}")
            if app_ui: app_ui.unlock_settings_button()
            return
            
        # ГЛАВНОЕ ИСПРАВЛЕНИЕ: Используем исправленный обработчик чата.
        # Это самый надежный метод, который решает проблему в корне.
        fixed_chat_handler = get_fixed_chat_handler()
        
        common_params = {
            "n_gpu_layers": -1, 
            "n_ctx": 4096, 
            "verbose": False,
            "chat_handler": fixed_chat_handler
        }
        
        default_llm = ChatLlamaCpp(model_path=model_path, temperature=0.1, **common_params)
        logger.info("Основная LLM модель успешно загружена и исправлена.")

        logger.info("Сборка графов LangGraph...")
        coding_graph = create_coding_graph(default_llm, mcp_client)
        research_graph = create_research_graph(default_llm, mcp_client)
        browser_graph = create_browser_graph(default_llm, mcp_client)
        dispatcher = create_dispatcher_graph(default_llm, coding_graph, research_graph, browser_graph)
        logger.info("Все графы успешно собраны.")

        logger.info("Инициализация голосового движка...")
        voice_controller = VoiceController(task_queue)
        voice_controller.set_ui_linker(app_ui)
        app_ui.set_voice_controller(voice_controller)
        logger.info("Голосовой движок успешно инициализирован и связан с UI.")

        if app_ui: app_ui.set_ui_busy(False)

        while not is_shutting_down:
            task = task_queue.get()
            if task is None: break

            if app_ui:
                app_ui.set_ui_busy(True, f"Обработка: {task[:50]}...")
                app_ui.log_to_widget(f"[Queue] Получена задача: {task}")

            final_state = dispatcher.invoke({"task": task})
            final_result = final_state.get("result", "Задача выполнена, но финальный результат отсутствует.")
            
            if app_ui:
                app_ui.log_to_widget(f"[Dispatcher] Финальный результат: {final_result}")
                app_ui.update_chat_with_final_result(final_result)
                app_ui.set_ui_busy(False)
            
            task_queue.task_done()

    except Exception as e:
        logger.critical(f"Критическая ошибка в фоновом потоке: {e}", exc_info=True)
        if app_ui: 
            app_ui.set_info_label(f"КРИТИЧЕСКАЯ ОШИБКА: {e}")
            app_ui.unlock_settings_button()

def main():
    global mcp_client, app_ui, voice_controller, is_shutting_down

    create_dirs()

    logger.info("Инициализация основного MCP клиента с Server Manager...")
    mcp_client = MCPClient.from_config_file("mcp_config.json")
    logger.info("Основной MCP клиент и все серверы инструментов успешно запущены.")

    def on_closing():
        global is_shutting_down
        if is_shutting_down: return
        is_shutting_down = True

        logger.info("Получен сигнал на завершение работы...")
        
        task_queue.put(None)

        if voice_controller:
            logger.info("Остановка голосового движка...")
            voice_controller.stop_listening()
        
        if mcp_client:
            logger.info("Закрытие сессий и остановка MCP серверов...")
            threading.Thread(target=lambda: asyncio.run(mcp_client.close_all_sessions()), daemon=True).start()
            logger.info("Команда на остановку серверов MCP отправлена.")

        if app_ui and app_ui.root.winfo_exists():
            app_ui.root.destroy()
        logger.info("Приложение завершило работу.")

    atexit.register(on_closing)

    processing_thread = threading.Thread(target=background_processing_loop, daemon=True)
    processing_thread.start()

    main_window = tk.Tk()
    app_ui = AppUI(main_window, task_queue)
    main_window.protocol("WM_DELETE_WINDOW", on_closing)
    main_window.mainloop()


if __name__ == "__main__":
    main()