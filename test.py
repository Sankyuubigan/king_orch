# --- ФИНАЛЬНЫЙ СКРИПТ С ИЗМЕНЕННЫМ "ПАСПОРТОМ" ---

import asyncio
import subprocess
import time
import json
import os
import websocket # <-- НАША НАДЕЖНАЯ БИБЛИОТЕКА

# --- Настройки ---
PORT = 8931
URL_TO_OPEN = "https://www.google.com/search?q=WE+DID+IT" # Победный запрос

# --- Пути ---
project_dir = os.path.dirname(os.path.abspath(__file__))
npx_path = os.path.join(project_dir, "modules", "nodejs", "npx.cmd")
cache_path = os.path.join(project_dir, "modules", "npm-cache")

# --- Команда для запуска сервера ---
command = [
    npx_path, "--yes", "--cache", cache_path,
    "@playwright/mcp@latest", f"--port={PORT}",
]

def run_sync_client():
    """
    Эта функция работает с новой библиотекой.
    """
    uri = f"ws://localhost:{PORT}/mcp"
    
    # --- КЛЮЧЕВОЕ ИЗМЕНЕНИЕ ЗДЕСЬ ---
    # Мы меняем наше имя на то, которое сервер, скорее всего, знает и доверяет.
    headers = {"X-MCP-Client-Name": "VSCode"} # <--- ПРИТВОРЯЕМСЯ VS CODE
    
    print(f">>> Клиент (новая библиотека) подключается к {uri}...")
    
    ws = websocket.create_connection(uri, header=headers)
    
    print("УСПЕШНО ПОДКЛЮЧИЛИСЬ!")
    
    message_id = "final-request-1"
    code_to_run = f'print(browser_navigate(url="{URL_TO_OPEN}"))'
    payload = {"id": message_id, "code": code_to_run}
    
    print(f"Отправляем команду на открытие сайта: {URL_TO_OPEN}")
    ws.send(json.dumps(payload))
    
    print("Ожидаем ответы от сервера...")
    while True:
        response_str = ws.recv()
        response_json = json.loads(response_str)
        print(f"ПОЛУЧЕН ОТВЕТ: {response_json}")
        if response_json.get('id') == message_id and response_json.get('result'):
            print(">>> Команда успешно выполнена сервером!")
            break
            
    ws.close()

async def main():
    server_process = None
    try:
        os.makedirs(cache_path, exist_ok=True)
        print("Запускаем сервер MCP...")
        
        server_process = subprocess.Popen(
            command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True
        )
        
        print("Ожидаем запуска сервера...")
        for _ in range(45):
            line = server_process.stdout.readline()
            if line:
                if "Listening on" in line:
                    print(f"ЛОГ СЕРВЕРА: {line.strip()}")
                    print(">>> Сервер готов к подключению!")
                    break
            time.sleep(0.5)
        else:
            raise RuntimeError("Server failed to start")

        loop = asyncio.get_running_loop()
        await loop.run_in_executor(None, run_sync_client)

        print("\nПОБЕДА! Браузер должен был открыться и перейти по ссылке.")
        print("Скрипт завершится через 15 секунд.")
        time.sleep(15)

    except Exception as e:
        print(f"Произошла ошибка: {e}")
    finally:
        if server_process and server_process.poll() is None:
            print("Останавливаем сервер MCP...")
            subprocess.run(['taskkill', '/F', '/T', '/PID', str(server_process.pid)], check=False, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            print("Сервер гарантированно остановлен.")

if __name__ == "__main__":
    asyncio.run(main())