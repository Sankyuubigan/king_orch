# run.py
import webview
import uvicorn
import multiprocessing
import time
import os
import sys
import requests
import logging

# Настройка логирования
logging.basicConfig(level=logging.INFO, format='%(asctime)s - %(levelname)s - %(message)s')
logger = logging.getLogger(__name__)

def check_server_health(max_attempts=40, delay=0.5):
    """Проверка здоровья сервера с таймаутом."""
    for attempt in range(max_attempts):
        try:
            response = requests.get("http://127.0.0.1:8000/health", timeout=2)
            if response.status_code == 200:
                health_data = response.json()
                logger.info(f"✅ Server health check passed: {health_data}")
                return True
        except requests.exceptions.RequestException:
            pass
        
        logger.info(f"⏳ Waiting for server... (attempt {attempt + 1}/{max_attempts})")
        time.sleep(delay)
    
    return False

def start_server():
    """Функция, которая будет выполняться в отдельном процессе."""
    try:
        from backend import create_app
        app = create_app()
        uvicorn.run(
            app, 
            host="127.0.0.1", 
            port=8000, 
            log_level="info",
            access_log=True
        )
    except Exception as e:
        logger.error(f"❌ Server startup failed: {e}")
        sys.exit(1)

if __name__ == '__main__':
    multiprocessing.freeze_support()
    
    logger.info("🚀 --- STARTING THE ORCHESTRATOR APPLICATION ---")
    
    try:
        # Запускаем сервер в отдельном процессе
        server_process = multiprocessing.Process(target=start_server, daemon=True)
        server_process.start()
        logger.info(f"✅ Server process started with PID: {server_process.pid}")
        
        # Ждем запуска сервера с проверкой здоровья
        logger.info("⏳ Waiting for server to initialize...")
        
        if not check_server_health():
            logger.error("❌ Server failed to start within timeout period")
            if server_process.is_alive():
                server_process.terminate()
                server_process.join()
            sys.exit(1)
        
        logger.info("✅ Server is healthy and ready!")
        
        # Получаем информацию о сервере
        try:
            info_response = requests.get("http://127.0.0.1:8000/info", timeout=5)
            if info_response.status_code == 200:
                info_data = info_response.json()
                logger.info(f"📊 Server info: {info_data}")
        except Exception as e:
            logger.warning(f"⚠️ Could not get server info: {e}")
        
        # Создаем и запускаем окно
        logger.info("🪟 Creating application window...")
        window = webview.create_window(
            title="The Orchestrator",
            url="http://127.0.0.1:8000",
            width=1200,
            height=800,
            min_size=(800, 600),
            resizable=True,
            shadow=True,
            on_top=False
        )
        
        logger.info("🎯 Starting webview...")
        webview.start(debug=False)  # Отключаем debug для продакшена
        
    except KeyboardInterrupt:
        logger.info("🛑 Application interrupted by user")
    except Exception as e:
        logger.error(f"❌ Application error: {e}")
    finally:
        # Очистка ресурсов
        logger.info("🧹 Cleaning up...")
        
        if 'server_process' in locals() and server_process.is_alive():
            logger.info("🛑 Terminating server process...")
            server_process.terminate()
            
            # Ждем завершения процесса
            try:
                server_process.join(timeout=5)
                if server_process.is_alive():
                    logger.warning("⚠️ Server process did not terminate gracefully, killing...")
                    server_process.kill()
            except Exception as e:
                logger.error(f"❌ Error terminating server process: {e}")
        
        logger.info("✅ Application has been closed successfully.")
        sys.exit(0)