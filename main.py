import tkinter as tk
import os
import threading
from queue import Queue
import logging
import atexit
import asyncio

# --- 1. Настройка окружения и логирования ---
os.environ['MCP_USE_ANONYMIZED_TELEMETRY'] = 'false'
from utils.logging_utils import setup_logging
setup_logging()

# --- 2. Импорт основных компонентов ---
from mcp_use import MCPClient
from ui import AppUI
from voice_engine.controller import VoiceController
from core_worker import CoreWorker
# ИЗМЕНЕНИЕ: Импортируем адаптер
from ui_adapter import UiAdapter

# --- 3. Глобальные переменные ---
task_queue = Queue()
# ИЗМЕНЕНИЕ: Новая очередь для команд UI
ui_update_queue = Queue() 
app_ui = None
mcp_client = None
voice_controller = None
core_worker = None
logger = logging.getLogger(__name__)

def create_dirs():
    # ... (код без изменений)
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

def main():
    """Главная функция, управляющая запуском и остановкой."""
    global mcp_client, app_ui, voice_controller, core_worker

    create_dirs()

    logger.info("Инициализация основного MCP клиента с Server Manager...")
    mcp_client = MCPClient.from_config_file("mcp_config.json")
    logger.info("Основной MCP клиент и все серверы инструментов успешно запущены.")

    # ИЗМЕНЕНИЕ: Создаем UI и передаем ему очередь обновлений
    main_window = tk.Tk()
    app_ui = AppUI(main_window, task_queue, ui_update_queue)

    # ИЗМЕНЕНИЕ: Создаем адаптер и передаем его в CoreWorker
    ui_adapter = UiAdapter(ui_update_queue)
    core_worker = CoreWorker(task_queue, mcp_client, ui_adapter)
    app_ui.set_core_worker(core_worker)
    
    processing_thread = threading.Thread(target=core_worker.run, daemon=True)
    processing_thread.start()

    logger.info("Инициализация голосового движка...")
    voice_controller = VoiceController(task_queue)
    voice_controller.set_ui_linker(app_ui)
    app_ui.set_voice_controller(voice_controller)
    logger.info("Голосовой движок успешно инициализирован и связан с UI.")

    def on_closing():
        logger.info("Получен сигнал на завершение работы...")
        
        if core_worker:
            core_worker.trigger_stop()
        
        if voice_controller:
            voice_controller.stop_listening()
        
        if mcp_client:
            logger.info("Закрытие сессий и остановка MCP серверов...")
            threading.Thread(target=lambda: asyncio.run(mcp_client.close_all_sessions()), daemon=True).start()
            logger.info("Команда на остановку серверов MCP отправлена.")

        if app_ui and app_ui.root.winfo_exists():
            app_ui.root.destroy()
        logger.info("Приложение завершило работу.")

    atexit.register(on_closing)
    main_window.protocol("WM_DELETE_WINDOW", on_closing)
    main_window.mainloop()


if __name__ == "__main__":
    main()