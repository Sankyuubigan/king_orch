import os
import sys
import subprocess
import shutil
import requests
import zipfile
import io

# --- Конфигурация портативного Node.js ---
NODE_VERSION = "v20.14.0"
NODE_OS = "win-x64"
NODE_FILENAME = f"node-{NODE_VERSION}-{NODE_OS}"
NODE_URL = f"https://nodejs.org/dist/{NODE_VERSION}/{NODE_FILENAME}.zip"
VENDOR_DIR = "vendor"
NODE_DIR = os.path.join(VENDOR_DIR, "nodejs")

# --- Полная конфигурация зависимостей ---
PYPI_PACKAGES = [
    "requests", "llama-cpp-python", "Pillow", "playwright", "fastapi",
    "uvicorn[standard]", "python-multipart", "tree-sitter",
    "sentence-transformers", "tree-sitter-languages", "vosk",
    "pyaudio", "sounddevice", "silero", "torch", "torchaudio", "torchvision"
]

CONFLICT_RESOLUTION_PACKAGES = [
    "Pillow", "typing-extensions", "jinja2"
]

GIT_APPS = [
    { "type": "python", "name": "mcp-server-playwright", "url": "https://github.com/Automata-Labs-team/MCP-Server-Playwright.git", "post_install_script": ["playwright", "install", "--with-deps"] },
    { "type": "node",   "name": "mcp-searxng", "url": "https://github.com/ihor-sokoliuk/mcp-searxng.git" },
    { "type": "node",   "name": "rag-mcp", "url": "https://github.com/qpd-v/mcp-ragdocs.git" },
    { "type": "python", "name": "chroma-mcp", "url": "https://github.com/chroma-core/chroma-mcp.git" },
    { "type": "python", "name": "mcp-code-runner", "url": "https://github.com/axliupore/mcp-code-runner.git" },
    { "type": "python", "name": "mcp-language-server", "url": "https://github.com/isaacphi/mcp-language-server.git" },
    { "type": "node",   "name": "ashra-mcp", "url": "https://github.com/getrupt/ashra-mcp.git" }
]

def install_portable_nodejs():
    """Скачивает и распаковывает портативную версию Node.js, если ее нет."""
    print("\n--- Шаг 0: Проверка наличия портативного Node.js ---")
    node_exe_path = os.path.join(NODE_DIR, 'node.exe')
    if os.path.exists(node_exe_path):
        print(f"Портативный Node.js уже найден в '{NODE_DIR}'. Пропускаю установку.")
        return True

    print(f"Портативный Node.js не найден. Начинаю загрузку с {NODE_URL}...")
    os.makedirs(NODE_DIR, exist_ok=True)
    
    try:
        response = requests.get(NODE_URL, stream=True)
        response.raise_for_status()
        
        with zipfile.ZipFile(io.BytesIO(response.content)) as z:
            # Распаковываем во временную папку, чтобы избежать проблем с вложенностью
            temp_extract_dir = os.path.join(VENDOR_DIR, "_temp_node")
            z.extractall(temp_extract_dir)
        
        # Перемещаем содержимое из вложенной папки в целевую
        source_dir = os.path.join(temp_extract_dir, NODE_FILENAME)
        for item in os.listdir(source_dir):
            shutil.move(os.path.join(source_dir, item), NODE_DIR)
        
        # Очистка
        shutil.rmtree(temp_extract_dir)
        
        print(f"Портативный Node.js успешно установлен в '{NODE_DIR}'.")
        return True
    except Exception as e:
        print(f"!!! КРИТИЧЕСКАЯ ОШИБКА при установке Node.js: {e}")
        return False

def run_command(command, cwd=None):
    print(f"\n>>> Running: {' '.join(command)} in {cwd or '.'}")
    try:
        use_shell = sys.platform == "win32"
        result = subprocess.run(command, check=False, cwd=cwd, shell=use_shell, capture_output=True, text=True, encoding='utf-8')
        if result.stdout: print(f"--- STDOUT ---\n{result.stdout.strip()}\n--------------")
        if result.returncode != 0:
            print(f"!!! ПРЕДУПРЕЖДЕНИЕ: Команда '{' '.join(command)}' завершилась с ошибкой.")
            if result.stderr: print(f"--- STDERR ---\n{result.stderr.strip()}\n--------------")
            return False
        return True
    except Exception as e:
        print(f"!!! КРИТИЧЕСКАЯ ОШИБКА при выполнении '{' '.join(command)}': {e}")
        return False

def main():
    if not install_portable_nodejs():
        print("\nНе удалось установить Node.js. Установка зависимостей Node.js будет пропущена.")
    
    # Определяем пути к портативным инструментам
    npm_cmd = os.path.join(NODE_DIR, 'npm.cmd')
    npx_cmd = os.path.join(NODE_DIR, 'npx.cmd')
    
    python_exe = sys.executable
    failed_packages = []
    
    print("\n--- Шаг 1: Установка базовых Python-пакетов с PyPI ---")
    if not run_command([python_exe, "-m", "pip", "install"] + PYPI_PACKAGES):
        failed_packages.append("базовые пакеты Python")

    print("\n--- Шаг 2: Клонирование и настройка внешних MCP-приложений ---")
    
    for app in GIT_APPS:
        app_name, app_url, app_type = app["name"], app["url"], app.get("type", "python")
        app_dir = os.path.join(VENDOR_DIR, app_name)
        success = True

        print(f"\n--- Настройка {app_name} ({app_type}) ---")

        if not os.path.exists(app_dir):
            if not run_command(["git", "clone", app_url, app_dir]):
                failed_packages.append(f"{app_name} (git clone)"); continue
        else:
            print(f"Папка '{app_dir}' уже существует. Пропускаем клонирование.")
        
        if app_type == "python":
            if os.path.exists(os.path.join(app_dir, 'requirements.txt')):
                if not run_command([python_exe, "-m", "pip", "install", "-r", os.path.join(app_dir, 'requirements.txt')]): success = False
            if os.path.exists(os.path.join(app_dir, 'pyproject.toml')) or os.path.exists(os.path.join(app_dir, 'setup.py')):
                 if not run_command([python_exe, "-m", "pip", "install", "."], cwd=app_dir): success = False

        elif app_type == "node":
            if not os.path.exists(npm_cmd):
                print(f"!!! Пропускаю установку {app_name}, так как портативный npm не найден."); success = False
            else:
                if not run_command([npm_cmd, "install"], cwd=app_dir): success = False
                package_json_path = os.path.join(app_dir, 'package.json')
                if success and os.path.exists(package_json_path):
                    with open(package_json_path, 'r', encoding='utf-8') as f:
                        if '"build"' in f.read():
                            if not run_command([npm_cmd, "run", "build"], cwd=app_dir): success = False
        
        if success and app.get("post_install_script"):
            # Используем портативный npx для playwright
            if not run_command([npx_cmd] + app["post_install_script"][1:]): success = False

        if not success: failed_packages.append(app_name)

    print("\n--- Шаг 3: Финальное разрешение конфликтов зависимостей ---")
    if not run_command([python_exe, "-m", "pip", "install", "--upgrade"] + CONFLICT_RESOLUTION_PACKAGES):
        failed_packages.append("разрешение конфликтов")

    print("\n\n--- Установка завершена! ---")
    if failed_packages:
        print("\n!!! ВНИМАНИЕ: Некоторые компоненты не удалось установить или настроить:")
        for pkg in failed_packages: print(f" - {pkg}")
    else:
        print("\nВсе зависимости успешно установлены! Теперь можно запускать main.py.")

if __name__ == "__main__":
    main()