# run.py
import webview
import uvicorn
import multiprocessing
import time
import os
import sys
import requests
import logging
import subprocess

# Настройка логирования
logging.basicConfig(level=logging.INFO, format='%(asctime)s - %(levelname)s - %(message)s')
logger = logging.getLogger(__name__)

def auto_build_frontend():
    """Автоматическая пересборка React приложения."""
    frontend_dir = "my-copilot-app"
    
    if not os.path.exists(frontend_dir):
        logger.warning(f"⚠️ Frontend directory not found: {frontend_dir}")
        return False
    
    try:
        logger.info(f"🔨 Building React application in {frontend_dir}...")
        
        # Проверяем наличие package.json
        package_json_path = os.path.join(frontend_dir, "package.json")
        if not os.path.exists(package_json_path):
            logger.warning(f"⚠️ package.json not found in {frontend_dir}")
            return False
        
        # Проверяем наличие node_modules
        node_modules_path = os.path.join(frontend_dir, "node_modules")
        if not os.path.exists(node_modules_path):
            logger.info("📦 Installing npm dependencies...")
            result = subprocess.run(
                ["npm", "install"], 
                cwd=frontend_dir, 
                capture_output=True, 
                text=True,
                timeout=300  # 5 минут на установку
            )
            
            if result.returncode != 0:
                logger.error(f"❌ npm install failed: {result.stderr}")
                return False
            
            logger.info("✅ Dependencies installed successfully")
        
        # Выполняем сборку
        logger.info("🏗️ Running npm run build...")
        result = subprocess.run(
            ["npm", "run", "build"], 
            cwd=frontend_dir, 
            capture_output=True, 
            text=True,
            timeout=120  # 2 минуты на сборку
        )
        
        if result.returncode == 0:
            logger.info("✅ React application built successfully!")
            
            # Проверяем, что папка dist создалась
            dist_path = os.path.join(frontend_dir, "dist")
            if os.path.exists(dist_path):
                files_count = len(os.listdir(dist_path))
                logger.info(f"📁 Build output: {files_count} files in {dist_path}")
                return True
            else:
                logger.warning("⚠️ Build completed but dist folder not found")
                return False
        else:
            logger.error(f"❌ Build failed: {result.stderr}")
            logger.info(f"Build stdout: {result.stdout}")
            return False
            
    except subprocess.TimeoutExpired:
        logger.error("❌ Build process timed out")
        return False
    except FileNotFoundError:
        logger.error("❌ npm not found. Please install Node.js and npm")
        return False
    except Exception as e:
        logger.error(f"❌ Build error: {e}")
        return False

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
        # 1. АВТОМАТИЧЕСКАЯ ПЕРЕСБОРКА FRONTEND
        logger.info("🔧 Step 1: Auto-building frontend...")
        build_success = auto_build_frontend()
        
        if not build_success:
            logger.warning("⚠️ Frontend build failed, but continuing anyway...")
            logger.info("💡 You can try building manually: cd my-copilot-app && npm run build")
        
        # 2. ЗАПУСК СЕРВЕРА
        logger.info("🔧 Step 2: Starting backend server...")
        server_process = multiprocessing.Process(target=start_server, daemon=True)
        server_process.start()
        logger.info(f"✅ Server process started with PID: {server_process.pid}")
        
        # 3. ОЖИДАНИЕ ГОТОВНОСТИ СЕРВЕРА
        logger.info("🔧 Step 3: Waiting for server to initialize...")
        
        if not check_server_health():
            logger.error("❌ Server failed to start within timeout period")
            if server_process.is_alive():
                server_process.terminate()
                server_process.join()
            sys.exit(1)
        
        logger.info("✅ Server is healthy and ready!")
        
        # 4. ПОЛУЧЕНИЕ ИНФОРМАЦИИ О СЕРВЕРЕ
        try:
            info_response = requests.get("http://127.0.0.1:8000/info", timeout=5)
            if info_response.status_code == 200:
                info_data = info_response.json()
                logger.info(f"📊 Server info: {info_data}")
        except Exception as e:
            logger.warning(f"⚠️ Could not get server info: {e}")
        
        # 5. СОЗДАНИЕ И ЗАПУСК ОКНА
        logger.info("🔧 Step 4: Creating application window...")
        window = webview.create_window(
            title="The Orchestrator 🎭",
            url="http://127.0.0.1:8000",
            width=1200,
            height=800,
            min_size=(800, 600),
            resizable=True,
            shadow=True,
            on_top=False
        )
        
        logger.info("🎯 Starting webview...")
        logger.info("🎉 Application ready! The chat should work now.")
        webview.start(debug=False)
        
    except KeyboardInterrupt:
        logger.info("🛑 Application interrupted by user")
    except Exception as e:
        logger.error(f"❌ Application error: {e}")
        import traceback
        traceback.print_exc()
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