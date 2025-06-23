# build_active_tools.py - НОВЫЙ СКРИПТ ДЛЯ СОЗДАНИЯ ДИНАМИЧЕСКОГО РЕЕСТРА ИНСТРУМЕНТОВ

import json
import socket
from urllib.parse import urlparse
import os

# Используем относительные пути для надежности
PROJECT_ROOT = os.path.dirname(os.path.abspath(__file__))
MASTER_CONFIG_PATH = os.path.join(PROJECT_ROOT, "tools_config.full.json")
ACTIVE_CONFIG_PATH = os.path.join(PROJECT_ROOT, "tools_config.json")

def check_port_is_active(host: str, port: int) -> bool:
    """
    Проверяет, открыт ли TCP-порт на указанном хосте.
    Возвращает True, если порт активен, иначе False.
    """
    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    # Устанавливаем короткий таймаут, чтобы не блокировать запуск надолго
    sock.settimeout(0.5)
    try:
        # Пробуем подключиться
        result = sock.connect_ex((host, port))
        # Если результат 0, порт открыт
        is_active = (result == 0)
        return is_active
    except (socket.timeout, socket.gaierror, ConnectionRefusedError):
        # Любая ошибка означает, что порт недоступен
        return False
    finally:
        sock.close()

def generate_active_tools_config():
    """
    "Пингует" все инструменты из мастер-конфига и создает новый файл
    конфигурации только с теми, которые реально работают.
    """
    print("\n" + "="*80)
    print("[ToolBuilder] Запуск сборки динамического реестра инструментов...")
    
    # Шаг 1: Проверяем наличие мастер-конфига
    if not os.path.exists(MASTER_CONFIG_PATH):
        print(f"[ToolBuilder] [ERROR] Мастер-конфиг '{MASTER_CONFIG_PATH}' не найден. Пропускаю сборку.")
        # Создаем пустой файл, чтобы избежать падения зависимых модулей
        with open(ACTIVE_CONFIG_PATH, "w", encoding="utf-8") as f:
            json.dump({}, f)
        return

    # Шаг 2: Читаем мастер-конфиг
    with open(MASTER_CONFIG_PATH, "r", encoding="utf-8") as f:
        master_config = json.load(f)

    active_tools = {}
    print("[ToolBuilder] Проверяю доступность серверов из мастер-списка...")

    # Шаг 3: Итерируемся и "пингуем" каждый инструмент
    for tool_name, config in master_config.items():
        url = config.get("url")
        if not url:
            print(f"[ToolBuilder] - '{tool_name}' пропущен (нет URL).")
            continue

        try:
            # Пропускаем проверку WebSocket URL, так как это другой протокол
            if url.startswith("ws://") or url.startswith("wss://"):
                 print(f"[ToolBuilder] - '{tool_name}' использует WebSocket. Добавляю в конфиг без проверки порта.")
                 active_tools[tool_name] = config
                 continue

            parsed_url = urlparse(url)
            host = parsed_url.hostname
            port = parsed_url.port
            
            if host and port:
                is_active = check_port_is_active(host, port)
                status = "АКТИВЕН" if is_active else "НЕДОСТУПЕН"
                print(f"[ToolBuilder] - Инструмент '{tool_name}' ({host}:{port})... {status}")
                if is_active:
                    active_tools[tool_name] = config
            else:
                print(f"[ToolBuilder] - '{tool_name}' имеет некорректный URL, не могу проверить.")
        except Exception as e:
            print(f"[ToolBuilder] - Ошибка при проверке '{tool_name}': {e}")
    
    # Шаг 4: Записываем результат в новый файл конфигурации
    with open(ACTIVE_CONFIG_PATH, "w", encoding="utf-8") as f:
        json.dump(active_tools, f, indent=2, ensure_ascii=False)
    
    print(f"[ToolBuilder] Сборка завершена. Найдено {len(active_tools)} активных инструментов.")
    print(f"[ToolBuilder] Актуальная конфигурация записана в '{ACTIVE_CONFIG_PATH}'.")
    print("="*80 + "\n")


if __name__ == "__main__":
    generate_active_tools_config()