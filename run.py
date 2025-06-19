# run.py - ВЕРСИЯ С АВТОМАТИЧЕСКОЙ СБОРКОЙ

import webview
import uvicorn
import multiprocessing
import time
import os
import sys
import requests
import logging
import subprocess

# Настройка логирования для ясного вывода
logging.basicConfig(level=logging.INFO, format='%(asctime)s - %(levelname)s - %(message)s')
logger = logging.getLogger(__name__)

def auto_build_frontend():
    """
    Автоматически устанавливает зависимости и собирает React-приложение.
    Возвращает True в случае успеха, False в случае неудачи.
    """
    frontend_dir = "my-copilot-app"
    
    if not os.path.exists(frontend_dir):
        logger.warning(f"⚠️ Каталог фронтенда не найден: {frontend_dir}")
        return False
    
    try:
        logger.info(f"🔨 Запускаю сборку React-приложения в каталоге {frontend_dir}...")
        
        # --- ШАГ 1: Проверка и установка зависимостей (npm install) ---
        node_modules_path = os.path.join(frontend_dir, "node_modules")
        if not os.path.exists(node_modules_path):
            logger.info("📦 Обнаружено отсутствие node_modules. Запускаю npm install...")
            install_process = subprocess.run(
                ["npm", "install"], 
                cwd=frontend_dir,       # Запускаем команду в каталоге фронтенда
                capture_output=True, 
                text=True,
                shell=True,             # Используем shell=True для совместимости с npm в Windows
                timeout=300             # Таймаут 5 минут
            )
            
            if install_process.returncode != 0:
                logger.error(f"❌ Ошибка при выполнении npm install:\n{install_process.stderr}")
                return False
            
            logger.info("✅ Зависимости успешно установлены.")
        
        # --- ШАГ 2: Сборка проекта (npm run build) ---
        logger.info("🏗️ Выполняю npm run build...")
        build_process = subprocess.run(
            ["npm", "run", "build"], 
            cwd=frontend_dir, 
            capture_output=True, 
            text=True,
            shell=True,
            timeout=120 # Таймаут 2 минуты
        )
        
        if build_process.returncode != 0:
            logger.error(f"❌ Ошибка сборки React-приложения:\n{build_process.stderr}")
            return False
            
        logger.info("✅ React-приложение успешно собрано!")
        
        # Проверяем, что папка dist действительно создана
        dist_path = os.path.join(frontend_dir, "dist")
        if os.path.exists(dist_path):
            logger.info(f"📁 Результат находится в папке: {dist_path}")
            return True
        else:
            logger.warning("⚠️ Сборка завершилась, но папка dist не найдена.")
            return False
            
    except subprocess.TimeoutExpired:
        logger.error("❌ Процесс сборки занял слишком много времени и был прерван.")
        return False
    except FileNotFoundError:
        logger.error("❌ Команда npm не найдена. Убедитесь, что Node.js и npm установлены и доступны в PATH.")
        return False
    except Exception as e:
        logger.error(f"❌ Произошла непредвиденная ошибка во время сборки: {e}")
        return False

def check_server_health(max_attempts=40, delay=0.5):
    """Проверка здоровья сервера с таймаутом."""
    for attempt in range(max_attempts):
        try:
            response = requests.get("http://127.0.0.1:8000/health", timeout=2)
            if response.status_code == 200:
                health_data = response.json()
                logger.info(f"✅ Проверка состояния сервера пройдена: {health_data}")
                return True
        except requests.exceptions.RequestException:
            pass
        
        logger.info(f"⏳ Ожидание запуска сервера... (попытка {attempt + 1}/{max_attempts})")
        time.sleep(delay)
    
    return False

def start_server():
    """Функция для запуска сервера FastAPI в отдельном процессе."""
    try:
        # Используем исправленную версию backend.py
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
        logger.error(f"❌ Ошибка при запуске сервера: {e}")
        sys.exit(1)

if __name__ == '__main__':
    multiprocessing.freeze_support()
    
    logger.info("🚀 --- ЗАПУСК ПРИЛОЖЕНИЯ THE ORCHESTRATOR ---")
    
    try:
        # --- ЭТАП 1: АВТОМАТИЧЕСКАЯ СБОРКА FRONTEND ---
        logger.info("🔧 Этап 1: Автоматическая сборка frontend...")
        build_success = auto_build_frontend()
        
        if not build_success:
            logger.error("❌ Сборка фронтенда провалилась. Дальнейший запуск невозможен.")
            logger.info("💡 Попробуйте запустить сборку вручную: cd my-copilot-app && npm run build")
            sys.exit(1) # Выходим, так как без фронтенда приложение бесполезно
        
        # --- ЭТАП 2: ЗАПУСК СЕРВЕРА ---
        logger.info("🔧 Этап 2: Запуск backend сервера...")
        server_process = multiprocessing.Process(target=start_server, daemon=True)
        server_process.start()
        logger.info(f"✅ Серверный процесс запущен с PID: {server_process.pid}")
        
        # --- ЭТАП 3: ОЖИДАНИЕ ГОТОВНОСТИ СЕРВЕРА ---
        logger.info("🔧 Этап 3: Ожидание инициализации сервера...")
        
        if not check_server_health():
            logger.error("❌ Сервер не смог запуститься за отведенное время.")
            if server_process.is_alive():
                server_process.terminate()
                server_process.join()
            sys.exit(1)
        
        logger.info("✅ Сервер исправен и готов к работе!")
        
        # --- ЭТАП 4: СОЗДАНИЕ И ЗАПУСК ОКНА ---
        logger.info("🔧 Этап 4: Создание окна приложения...")
        window = webview.create_window(
            title="The Orchestrator 🎭",
            url="http://127.0.0.1:8000",
            width=1200,
            height=800,
            min_size=(800, 600),
        )
        
        logger.info("🎉 Приложение готово! Чат должен работать.")
        webview.start(debug=False)
        
    except Exception as e:
        logger.error(f"❌ Критическая ошибка в приложении: {e}")
        import traceback
        traceback.print_exc()
    finally:
        # Корректное завершение всех процессов
        logger.info("🧹 Завершение работы и очистка ресурсов...")
        if 'server_process' in locals() and server_process.is_alive():
            logger.info("🛑 Завершаю процесс сервера...")
            server_process.terminate()
            server_process.join(timeout=5)
            if server_process.is_alive():
                logger.warning("⚠️ Сервер не завершился штатно, принудительное завершение...")
                server_process.kill()
        
        logger.info("✅ Приложение успешно закрыто.")
        sys.exit(0)