# run.py

import subprocess
import sys
import time
import logging
import threading
import webview
import os

# --- НАСТРОЙКИ ---
APP_FILE = "app.py"
STREAMLIT_URL = "http://localhost:8501"

logging.basicConfig(level=logging.INFO, format='%(asctime)s - %(levelname)s - [run.py] - %(message)s')
logger = logging.getLogger(__name__)

def run_streamlit():
    """Просто запускает Streamlit. Вся логика логов теперь внутри app.py."""
    command = [sys.executable, "-m", "streamlit", "run", APP_FILE, "--server.headless", "true"]
    # Мы снова "глушим" вывод, потому что он нам больше не нужен, все логи будут в интерфейсе.
    subprocess.run(command, check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)

# --- ГЛАВНЫЙ БЛОК ---
if __name__ == '__main__':
    try:
        streamlit_thread = threading.Thread(target=run_streamlit, daemon=True)
        streamlit_thread.start()
        logger.info("Ожидание запуска Streamlit (5 секунд)...")
        time.sleep(5)

        if not streamlit_thread.is_alive():
            logger.error("Поток Streamlit неожиданно завершился. Это не должно было произойти.")
            input("Нажмите Enter для выхода...")
            sys.exit(1)

        window = webview.create_window("The Orchestrator 🎭", STREAMLIT_URL, width=1200, height=800)
        
        def on_loaded():
            window.evaluate_js("""
                const style = document.createElement('style');
                style.innerHTML = `* { user-select: text !important; -webkit-user-select: text !important; }`;
                document.head.appendChild(style);
            """)
        window.events.loaded += on_loaded
        
        webview.start()

    except Exception as e:
        logger.error(f"❌ КРИТИЧЕСКАЯ ОШИБКА В RUN.PY: {e}", exc_info=True)
        input("Нажмите Enter для выхода...")
    finally:
        logger.info("✅ Приложение закрыто.")
        sys.exit(0)