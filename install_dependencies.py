import os
import sys
import subprocess
import shutil

# --- Конфигурация зависимостей ---

PYPI_PACKAGES = [
    "requests", "llama-cpp-python", "Pillow", "playwright", "fastapi",
    "uvicorn[standard]", "python-multipart", "chroma-mcp", "tree-sitter",
    "sentence-transformers", "tree-sitter-languages", "vosk", "torch",
    "silero", "pyaudio", "sounddevice"
]

GIT_APPS = [
    # Python-based servers
    { "type": "python", "name": "mcp-server-playwright", "url": "https://github.com/Automata-Labs-team/MCP-Server-Playwright.git", "post_install_script": ["playwright", "install", "--with-deps"] },
    { "type": "python", "name": "FileScopeMCP", "url": "https://github.com/admica/FileScopeMCP.git" },
    { "type": "python", "name": "mcp-language-server", "url": "https://github.com/isaacphi/mcp-language-server.git" },
    { "type": "python", "name": "mcp-searxng", "url": "https://github.com/ihor-sokoliuk/mcp-searxng.git" },
    { "type": "python", "name": "mcp-code-runner", "url": "https://github.com/axliupore/mcp-code-runner.git" },
    { "type": "python", "name": "mcp-text-editor", "url": "https://github.com/tumf/mcp-text-editor.git" },
    
    # --- ИЗМЕНЕНО: Добавлен Node.js проект с правильной логикой ---
    { "type": "node", "name": "ashra-mcp", "url": "https://github.com/getrupt/ashra-mcp.git" }
]

# --- Логика скрипта ---

def command_exists(cmd):
    """Проверяет, доступна ли команда в системном PATH."""
    return shutil.which(cmd) is not None

def run_command(command, cwd=None, env=None):
    """Запускает команду в терминале и проверяет на ошибки."""
    print(f"\n>>> Running: {' '.join(command)} in {cwd or '.'}")
    try:
        subprocess.run(command, check=True, cwd=cwd, env=env)
    except subprocess.CalledProcessError as e:
        print(f"!!! ОШИБКА: Команда '{' '.join(command)}' не удалась: {e}")
        sys.exit(1)
    except FileNotFoundError:
        print(f"!!! ОШИБКА: Команда '{command[0]}' не найдена. Убедитесь, что она установлена и доступна в PATH.")
        sys.exit(1)

def main():
    print("--- Начало установки зависимостей ---")
    python_exe = sys.executable
    
    print("\n--- Шаг 1: Установка базовых Python-пакетов с PyPI ---")
    run_command([python_exe, "-m", "pip", "install"] + PYPI_PACKAGES)

    print("\n--- Шаг 2: Клонирование и настройка внешних MCP-приложений ---")
    vendor_dir = "vendor"
    if not os.path.exists(vendor_dir):
        os.makedirs(vendor_dir)
    
    for app in GIT_APPS:
        app_name = app["name"]
        app_url = app["url"]
        app_type = app["type"]
        app_dir = os.path.join(vendor_dir, app_name)

        print(f"\n--- Настройка {app_name} ({app_type}) ---")

        if not os.path.exists(app_dir):
            run_command(["git", "clone", app_url, app_dir])
        else:
            print(f"Папка '{app_dir}' уже существует. Пропускаем клонирование.")
        
        if app_type == "python":
            requirements_file = os.path.join(app_dir, 'requirements.txt')
            if os.path.exists(requirements_file):
                run_command([python_exe, "-m", "pip", "install", "-r", requirements_file], cwd=app_dir)
            if app.get("post_install_script"):
                run_command([python_exe, "-m"] + app["post_install_script"])
        
        elif app_type == "node":
            if not command_exists("node") or not command_exists("npm"):
                print("!!! КРИТИЧЕСКАЯ ОШИБКА: Node.js и npm не найдены.")
                print("Пожалуйста, установите Node.js (https://nodejs.org/) и убедитесь, что он доступен в PATH.")
                sys.exit(1)
            
            # Устанавливаем npm-зависимости и собираем проект
            print(f"Найден package.json. Устанавливаем зависимости для {app_name}...")
            run_command(["npm", "install"], cwd=app_dir)
            
            print(f"Собираем проект {app_name}...")
            run_command(["npm", "run", "build"], cwd=app_dir)

    print("\n\n--- Установка всех зависимостей успешно завершена! ---")
    print("Теперь вы можете запускать основной проект.")

if __name__ == "__main__":
    main()