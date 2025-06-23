import os
import sys
import subprocess
import shutil

# --- Полная конфигурация зависимостей ---

PYPI_PACKAGES = [
    "requests", "llama-cpp-python", "Pillow", "playwright", "fastapi",
    "uvicorn[standard]", "python-multipart", "tree-sitter",
    "sentence-transformers", "tree-sitter-languages", "vosk", "torch",
    "silero", "pyaudio", "sounddevice"
]

GIT_APPS = [
    { "type": "python", "name": "mcp-server-playwright", "url": "https://github.com/Automata-Labs-team/MCP-Server-Playwright.git", "post_install_script": ["playwright", "install", "--with-deps"] },
    { "type": "python", "name": "FileScopeMCP", "url": "https://github.com/admica/FileScopeMCP.git" },
    { "type": "python", "name": "mcp-language-server", "url": "https://github.com/isaacphi/mcp-language-server.git" },
    { "type": "python", "name": "mcp-searxng", "url": "https://github.com/ihor-sokoliuk/mcp-searxng.git" },
    { "type": "python", "name": "mcp-code-runner", "url": "https://github.com/axliupore/mcp-code-runner.git" },
    { "type": "python", "name": "mcp-text-editor", "url": "https://github.com/tumf/mcp-text-editor.git" },
    { "type": "python", "name": "rag-mcp", "url": "https://github.com/hannesrudolph/rag-mcp.git" },
    { "type": "python", "name": "chroma-mcp", "url": "https://github.com/modelcontext/chroma-mcp.git" },
    { "type": "node", "name": "ashra-mcp", "url": "https://github.com/getrupt/ashra-mcp.git" }
]

def command_exists(cmd):
    return shutil.which(cmd) is not None

def run_command(command, cwd=None):
    print(f"\n>>> Running: {' '.join(command)} in {cwd or '.'}")
    try:
        # Запускаем и ждем завершения, но не вылетаем при ошибке
        result = subprocess.run(command, check=False, cwd=cwd, shell=False, capture_output=True, text=True, encoding='utf-8')
        if result.returncode != 0:
            print(f"!!! ПРЕДУПРЕЖДЕНИЕ: Команда '{' '.join(command)}' завершилась с ошибкой.")
            print(f"--- STDOUT ---\n{result.stdout}\n--------------")
            print(f"--- STDERR ---\n{result.stderr}\n--------------")
            return False
        return True
    except FileNotFoundError:
        print(f"!!! ОШИБКА: Команда '{command[0]}' не найдена. Убедитесь, что она установлена и доступна в PATH.")
        return False

def main():
    print("--- Начало установки зависимостей ---")
    python_exe = sys.executable
    failed_packages = []
    
    print("\n--- Шаг 1: Установка базовых Python-пакетов с PyPI ---")
    if not run_command([python_exe, "-m", "pip", "install"] + PYPI_PACKAGES):
        failed_packages.append("базовые пакеты Python")

    print("\n--- Шаг 2: Клонирование и настройка внешних MCP-приложений ---")
    vendor_dir = "vendor"
    if not os.path.exists(vendor_dir):
        os.makedirs(vendor_dir)
    
    for app in GIT_APPS:
        app_name = app["name"]
        app_url = app["url"]
        app_type = app.get("type", "python")
        app_dir = os.path.join(vendor_dir, app_name)
        success = True

        print(f"\n--- Настройка {app_name} ({app_type}) ---")

        if not os.path.exists(app_dir):
            if not run_command(["git", "clone", app_url, app_dir]):
                failed_packages.append(f"{app_name} (git clone)")
                continue
        else:
            print(f"Папка '{app_dir}' уже существует. Пропускаем клонирование.")
        
        if app_type == "python":
            requirements_file = os.path.join(app_dir, 'requirements.txt')
            if os.path.exists(requirements_file):
                if not run_command([python_exe, "-m", "pip", "install", "-r", requirements_file], cwd=app_dir):
                    success = False
            
            if os.path.exists(os.path.join(app_dir, 'pyproject.toml')):
                 if not run_command([python_exe, "-m", "pip", "install", "."], cwd=app_dir):
                     success = False

        elif app_type == "node":
            if not command_exists("npm"):
                print("!!! ПРЕДУПРЕЖДЕНИЕ: npm не найден. Пропускаем установку. Пожалуйста, установите Node.js.")
                success = False
            else:
                if not run_command(["npm", "install"], cwd=app_dir):
                    success = False
                else:
                    package_json_path = os.path.join(app_dir, 'package.json')
                    if os.path.exists(package_json_path):
                        with open(package_json_path) as f:
                            if '"build"' in f.read():
                                if not run_command(["npm", "run", "build"], cwd=app_dir):
                                    success = False
        
        if success and app.get("post_install_script"):
            if not run_command([python_exe, "-m"] + app["post_install_script"]):
                success = False

        if not success:
            failed_packages.append(app_name)

    print("\n\n--- Установка завершена! ---")
    if failed_packages:
        print("\n!!! ВНИМАНИЕ: Некоторые компоненты не удалось установить:")
        for pkg in failed_packages:
            print(f" - {pkg}")
        print("Пожалуйста, просмотрите лог выше, чтобы понять причину.")
    else:
        print("Все зависимости успешно установлены!")

if __name__ == "__main__":
    main()