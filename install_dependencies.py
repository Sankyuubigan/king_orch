import os
import sys
import subprocess
import shutil
import requests
import zipfile
import io

# --- Конфигурация портативных сред ---
VENDOR_DIR = "vendor"

# Python
PYTHON_VERSION = "3.11.9"
PYTHON_DIR = os.path.join(VENDOR_DIR, "python")
PYTHON_EXE = os.path.join(PYTHON_DIR, "python.exe")
PYTHON_URL = f"https://www.python.org/ftp/python/{PYTHON_VERSION}/python-{PYTHON_VERSION}-embed-amd64.zip"
GET_PIP_URL = "https://bootstrap.pypa.io/get-pip.py"

# Node.js
NODE_VERSION = "v20.14.0"
NODE_DIR = os.path.join(VENDOR_DIR, "nodejs")
NODE_EXE = os.path.join(NODE_DIR, "node.exe")
NPM_CMD = os.path.join(NODE_DIR, "npm.cmd")
NPX_CMD = os.path.join(NODE_DIR, "npx.cmd")
NODE_FILENAME = f"node-{NODE_VERSION}-win-x64"
NODE_URL = f"https://nodejs.org/dist/{NODE_VERSION}/{NODE_FILENAME}.zip"

# --- Конфигурация зависимостей ---
# ИЗМЕНЕНО: markitdown-mcp добавлен как основной пакет
PYPI_PACKAGES = [
    "requests", "llama-cpp-python", "Pillow", "playwright", "fastapi",
    "uvicorn[standard]", "python-multipart", "tree-sitter",
    "sentence-transformers", "tree-sitter-languages", "vosk",
    "pyaudio", "sounddevice", "silero", "torch", "torchaudio", "torchvision",
    "markitdown-mcp" 
]

GIT_APPS = [
    { "type": "node",   "name": "mcp-searxng", "url": "https://github.com/ihor-sokoliuk/mcp-searxng.git" },
    { "type": "node",   "name": "rag-mcp", "url": "https://github.com/qpd-v/mcp-ragdocs.git" },
    { "type": "python", "name": "chroma-mcp", "url": "https://github.com/chroma-core/chroma-mcp.git" },
    { "type": "python", "name": "mcp-code-runner", "url": "https://github.com/axliupore/mcp-code-runner.git" },
    { "type": "python", "name": "mcp-language-server", "url": "https://github.com/isaacphi/mcp-language-server.git" },
]

def run_command(command, cwd=None, env=None):
    print(f"\n>>> Running: {' '.join(command)} in {cwd or '.'}")
    try:
        use_shell = sys.platform == "win32"
        # ИСПРАВЛЕНО: Добавлен errors='ignore' для решения проблемы с кодировкой
        result = subprocess.run(command, check=False, cwd=cwd, shell=use_shell, capture_output=True, text=True, encoding='utf-8', errors='ignore', env=env)
        if result.stdout: print(f"--- STDOUT ---\n{result.stdout.strip()}\n--------------")
        if result.returncode != 0:
            print(f"!!! ПРЕДУПРЕЖДЕНИЕ: Команда '{' '.join(command)}' завершилась с ошибкой.")
            if result.stderr: print(f"--- STDERR ---\n{result.stderr.strip()}\n--------------")
            return False
        return True
    except Exception as e:
        print(f"!!! КРИТИЧЕСКАЯ ОШИБКА при выполнении '{' '.join(command)}': {e}")
        return False

def install_portable_env(name, version, url, target_dir, exe_check, post_install_func=None):
    print(f"\n--- Шаг 0: Проверка наличия портативного {name} ---")
    if os.path.exists(exe_check):
        print(f"Портативный {name} уже найден в '{target_dir}'. Пропускаю.")
        return True
    
    print(f"Портативный {name} не найден. Начинаю загрузку...")
    os.makedirs(target_dir, exist_ok=True)
    
    try:
        response = requests.get(url, stream=True)
        response.raise_for_status()
        
        with zipfile.ZipFile(io.BytesIO(response.content)) as z:
            if name == "Node.js":
                temp_dir = os.path.join(VENDOR_DIR, "_temp_node")
                z.extractall(temp_dir)
                source_dir = os.path.join(temp_dir, f"node-{version}-win-x64")
                for item in os.listdir(source_dir):
                    shutil.move(os.path.join(source_dir, item), target_dir)
                shutil.rmtree(temp_dir)
            else: # Python
                z.extractall(target_dir)
        
        print(f"Портативный {name} успешно установлен в '{target_dir}'.")
        
        if post_install_func:
            print(f"Выполняю пост-установочные шаги для {name}...")
            return post_install_func()
            
        return True
    except Exception as e:
        print(f"!!! КРИТИЧЕСКАЯ ОШИБКА при установке {name}: {e}")
        return False

def install_pip_for_portable_python():
    get_pip_path = os.path.join(PYTHON_DIR, "get-pip.py")
    pip_exe_path = os.path.join(PYTHON_DIR, "Scripts", "pip.exe")
    if os.path.exists(pip_exe_path):
        print("pip уже установлен для портативного Python.")
        return True
        
    print("Скачиваю get-pip.py...")
    response = requests.get(GET_PIP_URL)
    with open(get_pip_path, 'wb') as f:
        f.write(response.content)
    
    print("Устанавливаю pip...")
    if run_command([PYTHON_EXE, get_pip_path]):
        os.remove(get_pip_path)
        print("pip успешно установлен.")
        return True
    else:
        print("!!! ОШИБКА: не удалось установить pip.")
        return False

def main():
    os.makedirs(VENDOR_DIR, exist_ok=True)
    
    if not install_portable_env("Python", PYTHON_VERSION, PYTHON_URL, PYTHON_DIR, PYTHON_EXE, install_pip_for_portable_python):
        sys.exit(1)
    if not install_portable_env("Node.js", NODE_VERSION, NODE_URL, NODE_DIR, NODE_EXE):
        sys.exit(1)

    print("\n--- Шаг 1: Установка базовых Python-пакетов в портативный Python ---")
    portable_pip_exe = os.path.join(PYTHON_DIR, "Scripts", "pip.exe")
    if not run_command([portable_pip_exe, "install"] + PYPI_PACKAGES):
        print("!!! ОШИБКА: Не удалось установить базовые пакеты Python.")

    print("\n--- Шаг 2: Клонирование и ИЗОЛИРОВАННАЯ настройка MCP-приложений ---")
    
    for app in GIT_APPS:
        app_name, app_url, app_type = app["name"], app["url"], app.get("type", "python")
        app_dir = os.path.join(VENDOR_DIR, app_name)

        print(f"\n--- Настройка {app_name} ({app_type}) ---")

        if not os.path.exists(app_dir):
            if not run_command(["git", "clone", app_url, app_dir]): continue
        else:
            print(f"Папка '{app_dir}' уже существует. Пропускаем клонирование.")
        
        if app_type == "python":
            venv_dir = os.path.join(app_dir, ".venv")
            if not os.path.exists(venv_dir):
                print(f"Создаю виртуальное окружение для {app_name}...")
                run_command([PYTHON_EXE, "-m", "venv", venv_dir])
            
            venv_pip_exe = os.path.join(venv_dir, "Scripts", "pip.exe")
            if os.path.exists(os.path.join(app_dir, 'requirements.txt')):
                print(f"Устанавливаю зависимости для {app_name} в его .venv...")
                run_command([venv_pip_exe, "install", "-r", "requirements.txt"], cwd=app_dir)

        elif app_type == "node":
            print(f"Устанавливаю npm-зависимости для {app_name}...")
            run_command([NPM_CMD, "install"], cwd=app_dir)
            package_json_path = os.path.join(app_dir, 'package.json')
            if os.path.exists(package_json_path):
                with open(package_json_path, 'r', encoding='utf-8') as f:
                    if '"build"' in f.read():
                        print(f"Собираю проект {app_name}...")
                        run_command([NPM_CMD, "run", "build"], cwd=app_dir)
    
    print("\n--- Шаг 3: Установка Playwright Browsers ---")
    # ИСПРАВЛЕНО: Правильная команда для установки браузеров через портативный npx
    run_command([NPX_CMD, "playwright", "install", "--with-deps"])

    print("\n\n--- Установка завершена! ---")
    print("Все зависимости должны быть установлены. Теперь можно запускать main.py.")

if __name__ == "__main__":
    main()