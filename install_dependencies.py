import os
import sys
import subprocess
import shutil
import requests
import zipfile
import io

# --- Конфигурация портативных сред ---
VENDOR_DIR = "vendor"
PYTHON_VERSION = "3.11.9"
PYTHON_DIR = os.path.join(VENDOR_DIR, "python")
# Путь к исполняемому файлу сделан абсолютным
PYTHON_EXE = os.path.abspath(os.path.join(PYTHON_DIR, "python.exe"))
PYTHON_URL = f"https://www.python.org/ftp/python/{PYTHON_VERSION}/python-{PYTHON_VERSION}-embed-amd64.zip"
GET_PIP_URL = "https://bootstrap.pypa.io/get-pip.py"

NODE_VERSION = "v20.14.0"
NODE_DIR = os.path.join(VENDOR_DIR, "nodejs")
# Пути к исполняемым файлам сделаны абсолютными
NODE_EXE = os.path.abspath(os.path.join(NODE_DIR, "node.exe"))
NPM_CMD = os.path.abspath(os.path.join(NODE_DIR, "npm.cmd"))
NPX_CMD = os.path.abspath(os.path.join(NODE_DIR, "npx.cmd"))
NODE_FILENAME = f"node-{NODE_VERSION}-win-x64"
NODE_URL = f"https://nodejs.org/dist/{NODE_VERSION}/{NODE_FILENAME}.zip"

# --- Зависимости ---
BUILD_DEPS = ["Cython", "numpy", "torch", "torchaudio", "torchvision"]
PYPI_PACKAGES = [
    "requests", "Pillow", "playwright", "fastapi", "uvicorn[standard]",
    "python-multipart", "tree-sitter", "sentence-transformers", "tree-sitter-languages",
    "vosk", "pyaudio", "sounddevice",
    "markitdown-mcp", "llama-cpp-python[server,llava]", "huggingface-hub",
    "transformers", "phonemizer", "scipy", "ruaccent"
]

GIT_APPS = [
    { "type": "node",   "name": "mcp-searxng", "url": "https://github.com/ihor-sokoliuk/mcp-searxng.git" },
    { "type": "node",   "name": "rag-mcp", "url": "https://github.com/qpd-v/mcp-ragdocs.git" },
    { "type": "python", "name": "chroma-mcp", "url": "https://github.com/chroma-core/chroma-mcp.git" },
    { "type": "python", "name": "mcp-code-runner", "url": "https://github.com/axliupore/mcp-code-runner.git" },
    { "type": "python", "name": "mcp-language-server", "url": "https://github.com/isaacphi/mcp-language-server.git" },
]

def run_command(command, cwd=None, env=None, ignore_errors=False):
    print(f"\n>>> Running: {' '.join(command)} in '{cwd or '.'}'")
    try:
        local_env = env if env is not None else os.environ.copy()
        node_dir_abs = os.path.abspath(NODE_DIR)
        local_env["PATH"] = f"{node_dir_abs}{os.pathsep}{local_env.get('PATH', '')}"

        use_shell = sys.platform == "win32"
        process = subprocess.Popen(
            command, cwd=cwd, shell=use_shell, stdout=sys.stdout, stderr=sys.stderr,
            text=True, encoding='utf-8', errors='ignore', env=local_env
        )
        process.wait()
        if process.returncode != 0:
            print(f"!!! ПРЕДУПРЕЖДЕНИЕ: Команда завершилась с кодом {process.returncode}.")
            return False if not ignore_errors else True
        return True
    except Exception as e:
        print(f"!!! КРИТИЧЕСКАЯ ОШИБКА при выполнении '{' '.join(command)}': {e}")
        return False

def setup_portable_python():
    print("\n--- Шаг 0: Настройка портативного Python ---")
    if not os.path.exists(PYTHON_EXE):
        print(f"Скачивание портативного Python {PYTHON_VERSION}...")
        try:
            response = requests.get(PYTHON_URL, stream=True)
            response.raise_for_status()
            with zipfile.ZipFile(io.BytesIO(response.content)) as z: z.extractall(PYTHON_DIR)
            print("Python успешно распакован.")
        except Exception as e:
            print(f"!!! ОШИБКА скачивания Python: {e}"); return False

    pth_file = os.path.join(PYTHON_DIR, f"python{PYTHON_VERSION.replace('.', '')[:3]}._pth")
    site_packages_path = "Lib\\site-packages"
    if os.path.exists(pth_file):
        with open(pth_file, "r+") as f:
            content = f.read()
            if site_packages_path not in content:
                print(f"Добавляю '{site_packages_path}' в '{pth_file}' для поиска модулей...")
                f.write(f"\n{site_packages_path}\n")
    else:
        print(f"!!! ПРЕДУПРЕЖДЕНИЕ: Не найден pth-файл по пути '{pth_file}'. pip может не работать.")

    print("Проверка и установка pip для портативного Python...")
    if not run_command([PYTHON_EXE, "-m", "pip", "--version"]):
        get_pip_path = os.path.join(PYTHON_DIR, "get-pip.py")
        print("Скачиваю get-pip.py...")
        try:
            response = requests.get(GET_PIP_URL)
            response.raise_for_status()
            with open(get_pip_path, 'wb') as f: f.write(response.content)
        except requests.RequestException as e:
            print(f"!!! ОШИБКА скачивания get-pip.py: {e}"); return False
        
        if not run_command([PYTHON_EXE, get_pip_path]):
            print("!!! Не удалось установить pip."); return False
        os.remove(get_pip_path)

    print("Обновляю pip, setuptools, wheel...")
    if not run_command([PYTHON_EXE, "-m", "pip", "install", "--upgrade", "pip", "setuptools", "wheel"]):
        print("!!! Не удалось обновить pip. Проверьте лог выше.")
        return False
    
    return True

def setup_portable_nodejs():
    print("\n--- Настройка портативного Node.js ---")
    if os.path.exists(NODE_EXE):
        print("Портативный Node.js уже найден."); return True
    print(f"Скачивание портативного Node.js {NODE_VERSION}...")
    os.makedirs(NODE_DIR, exist_ok=True)
    try:
        response = requests.get(NODE_URL, stream=True)
        response.raise_for_status()
        with zipfile.ZipFile(io.BytesIO(response.content)) as z:
            temp_dir = os.path.join(VENDOR_DIR, "_temp_node")
            z.extractall(temp_dir)
            source_dir = os.path.join(temp_dir, NODE_FILENAME)
            for item in os.listdir(source_dir): shutil.move(os.path.join(source_dir, item), NODE_DIR)
            shutil.rmtree(temp_dir)
        print("Node.js успешно установлен."); return True
    except Exception as e:
        print(f"!!! ОШИБКА скачивания Node.js: {e}"); return False

def main():
    os.makedirs(VENDOR_DIR, exist_ok=True)
    if not setup_portable_python(): sys.exit("Критическая ошибка: не удалось настроить портативный Python.")
    if not setup_portable_nodejs(): sys.exit("Критическая ошибка: не удалось настроить портативный Node.js.")

    print("\n--- Шаг 1: Установка зависимостей в портативный Python ---")
    
    print("\n--- Этап 1.1: Установка зависимостей для сборки (Cython, numpy, torch) ---")
    if not run_command([PYTHON_EXE, "-m", "pip", "install", "--upgrade"] + BUILD_DEPS):
        sys.exit("Не удалось установить базовые зависимости для сборки.")
    
    print("\n--- Этап 1.2: Установка основных пакетов ---")
    if not run_command([PYTHON_EXE, "-m", "pip", "install", "--upgrade"] + PYPI_PACKAGES):
        sys.exit("Не удалось установить основные пакеты.")


    print("\n--- Шаг 2: Клонирование и настройка MCP-приложений ---")
    for app in GIT_APPS:
        app_name, app_url, app_type = app["name"], app["url"], app.get("type", "python")
        app_dir = os.path.join(VENDOR_DIR, app_name)
        print(f"\n--- Настройка {app_name} ({app_type}) ---")
        if not os.path.exists(app_dir):
            if not run_command(["git", "clone", app_url, app_dir]): continue
        else: print(f"Папка '{app_dir}' уже существует.")
        
        if app_type == "python":
            requirements_path = os.path.join(app_dir, 'requirements.txt')
            if os.path.exists(requirements_path):
                run_command([PYTHON_EXE, "-m", "pip", "install", "-r", requirements_path], cwd=app_dir)
        elif app_type == "node":
            if run_command([NPM_CMD, "install"], cwd=app_dir):
                # ИСПРАВЛЕНО: Добавляем автоматическое исправление уязвимостей.
                # Запускаем `npm audit fix`. Мы игнорируем ошибки (ignore_errors=True),
                # так как иногда `audit fix` не может исправить всё, но это не должно
                # останавливать весь процесс установки.
                print(f"--- Запуск аудита и исправления уязвимостей для {app_name} ---")
                run_command([NPM_CMD, "audit", "fix"], cwd=app_dir, ignore_errors=True)

            package_json_path = os.path.join(app_dir, 'package.json')
            if os.path.exists(package_json_path):
                with open(package_json_path, 'r', encoding='utf-8') as f:
                    if '"build"' in f.read(): run_command([NPM_CMD, "run", "build"], cwd=app_dir)

    print("\n" + "="*80)
    print("--- Шаг 3: УСТАНОВКА БРАУЗЕРОВ PLAYWRIGHT (МОЖЕТ ЗАНЯТЬ 5-15 МИНУТ) ---")
    print("="*80)
    run_command([NPX_CMD, "--yes", "playwright", "install", "--with-deps"])
    
    print("\n\n--- Установка завершена! ---")
    print("Все зависимости установлены. Теперь можно запускать main.py.")

if __name__ == "__main__":
    main()