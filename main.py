import os
import signal
import subprocess
import threading
import time
import gradio as gr
import webview
import json
from ollama import Client as OllamaChatClient
from ollama import list as ollama_list
from stagehand import Stagehand

# =========================================================================
# [!!! НАСТРОЙКИ !!!]
OLLAMA_PATH = r"D:\Projects\universal_orchestrator\ollama_runtime\ollama.exe"
DEFAULT_MODEL = "llama3:8b"
PROMPT_FILE = 'system_prompt.md'
GRADIO_SERVER_PORT = 7860
# =========================================================================

# --- Глобальные переменные ---
SYSTEM_PROMPT = "You are a helpful assistant."
agent = None
ollama_chat_client = None
window = None
ollama_process = None
app_settings = {"model": DEFAULT_MODEL, "headless": True}

# --- Функции управления приложением ---

def load_system_prompt():
    global SYSTEM_PROMPT
    try:
        with open(PROMPT_FILE, 'r', encoding='utf-8') as f:
            SYSTEM_PROMPT = f.read()
        print(f"System prompt loaded from {PROMPT_FILE}")
    except FileNotFoundError:
        print(f"ERROR: System prompt file not found at '{PROMPT_FILE}'!")

def get_ollama_models():
    """Простая, но вызываемая в нужный момент функция."""
    try:
        models = ollama_list()['models']
        return [model['name'] for model in models]
    except Exception as e:
        print(f"Error fetching models: {e}")
        return []

def run_ollama_server():
    global ollama_process
    print("Starting Ollama Server...")
    ollama_process = subprocess.Popen([OLLAMA_PATH, "serve"], creationflags=subprocess.CREATE_NO_WINDOW)
    print(f"Ollama Server started with PID: {ollama_process.pid}")

def update_clients(model_name, show_browser):
    global agent, ollama_chat_client, app_settings
    print(f"Updating clients: Model -> {model_name}, Show Browser -> {show_browser}")
    app_settings["model"] = model_name
    app_settings["headless"] = not show_browser # show_browser=True -> headless=False
    try:
        agent = Stagehand(model=f"ollama/{model_name}", browser_options={"headless": app_settings["headless"]})
        ollama_chat_client = OllamaChatClient(host='http://127.0.0.1:11434')
        return f"✅ Настройки применены! Активная модель: `{model_name}`."
    except Exception as e:
        print(f"Error updating clients: {e}")
        return f"❌ Ошибка при обновлении клиентов: {e}"

def handle_model_creation(file_obj, model_name):
    if not file_obj or not model_name:
        yield "❌ Ошибка: Выберите файл и укажите имя.", gr.Dropdown(), gr.Button(interactive=True)
        return

    yield "⏳ Идет добавление... Это может занять много минут.", gr.Dropdown(), gr.Button(interactive=False)
    
    model_name = model_name.strip().lower().replace(" ", "-")
    gguf_path = file_obj.name
    modelfile_path = f"./{model_name}.modelfile"
    
    try:
        with open(modelfile_path, 'w', encoding='utf-8') as f:
            f.write(f'FROM "{os.path.abspath(gguf_path)}"')
        
        command = [OLLAMA_PATH, "create", model_name, "-f", modelfile_path]
        print(f"Running long process: {' '.join(command)}")
        
        result = subprocess.run(command, capture_output=True, text=True, check=True, encoding='utf-8')
        print(result.stdout)
        os.remove(modelfile_path)
        
        print("Model created. Waiting 5 seconds for Ollama server to settle...")
        time.sleep(5)
        
        updated_models = get_ollama_models()
        yield f"✅ Модель '{model_name}' успешно добавлена!", gr.Dropdown(choices=updated_models, value=model_name), gr.Button(interactive=True)

    except Exception as e:
        yield f"❌ Ошибка: {e}", gr.Dropdown(), gr.Button(interactive=True)

# --- Основная логика чата ---
def chat_function(message, history):
    history = history or []
    history.append([message, None])
    
    # Собираем историю для первого запроса к модели
    conversation_for_decision = [{'role': 'system', 'content': SYSTEM_PROMPT}]
    for user_msg, assistant_msg in history[:-1]: # Все, кроме последнего пустого ответа
        conversation_for_decision.append({'role': 'user', 'content': user_msg})
        if assistant_msg:
            clean_assistant_msg = assistant_msg.replace("🔎 ", "", 1)
            if not (clean_assistant_msg.startswith("🤖") or clean_assistant_msg.startswith("✅")):
                conversation_for_decision.append({'role': 'assistant', 'content': clean_assistant_msg})
    conversation_for_decision.append({'role': 'user', 'content': message})

    try:
        # Шаг 1: Получаем решение от модели
        response = ollama_chat_client.chat(model=app_settings["model"], messages=conversation_for_decision)
        model_response_content = response['message']['content']
        print(f"Model initial response: {model_response_content}")
        
        # Шаг 2: Пытаемся распознать команду
        try:
            tool_call = json.loads(model_response_content)
            if isinstance(tool_call, dict) and tool_call.get("tool") == "stagehand_search":
                query = tool_call.get("query")
                history[-1][1] = "🤖 *Модель решила использовать Stagehand...*"
                yield history
                
                # Шаг 3: Выполняем поиск
                stagehand_result = agent.invoke(goal=query, url="https://www.google.com")
                history[-1][1] = "✅ *Поиск завершен. Формулирую ответ...*"
                yield history
                
                # Шаг 4: Формируем финальный ответ
                search_answer = stagehand_result.get('answer', 'Не удалось извлечь ответ.')
                final_prompt = [
                    {'role': 'system', 'content': 'Ты - ассистент, который отвечает на вопрос пользователя, основываясь на предоставленных результатах поиска.'},
                    {'role': 'user', 'content': message},
                    {'role': 'system', 'content': f"Результаты поиска по запросу '{query}':\n{search_answer}"}
                ]
                
                final_response_stream = ollama_chat_client.chat(model=app_settings["model"], messages=final_prompt, stream=True)
                
                full_response = ""
                for chunk in final_response_stream:
                    full_response += chunk['message']['content']
                    history[-1][1] = f"🔎 {full_response}"
                    yield history
                return
        except (json.JSONDecodeError, TypeError):
            # Если это не JSON - это обычный ответ
            history[-1][1] = model_response_content
            yield history
            
    except Exception as e:
        print(f"CRITICAL ERROR in chat_function: {e}")
        import traceback
        traceback.print_exc()
        history[-1][1] = f"Произошла критическая ошибка: {e}"
        yield history

# --- Сборка интерфейса Gradio ---
def create_gradio_app(models):
    with gr.Blocks(theme=gr.themes.Soft(), title="The Orchestrator") as app:
        gr.Markdown("# The Orchestrator (Agent Edition)")
        
        with gr.Tabs():
            with gr.TabItem("Чат"):
                with gr.Row():
                    with gr.Column(scale=1):
                        gr.Markdown("### Настройки чата")
                        status_display = gr.Markdown("✅ Статус: Готов")
                        model_dropdown = gr.Dropdown(label="Активная модель", choices=models, value=app_settings["model"])
                        headless_checkbox = gr.Checkbox(label="Показывать браузер Stagehand", value=False)
                        apply_button = gr.Button("Применить настройки")
                    with gr.Column(scale=3):
                        chatbot = gr.Chatbot(height=600, label="Чат")
                        msg_textbox = gr.Textbox(placeholder="Спроси меня о чем-нибудь...", container=False, scale=7)
                        send_button = gr.Button("Отправить")

            with gr.TabItem("Управление моделями"):
                gr.Markdown("### Добавить модель из GGUF файла")
                with gr.Row():
                    gguf_file_upload = gr.File(label="1. Выберите .gguf файл", file_types=['.gguf'])
                    new_model_name = gr.Textbox(label="2. Придумайте короткое имя")
                create_model_button = gr.Button("3. Добавить модель", variant="primary")
                creation_status = gr.Markdown("")

        def submit_message(message, history):
            for h in chat_function(message, history):
                yield h, ""

        send_button.click(fn=submit_message, inputs=[msg_textbox, chatbot], outputs=[chatbot, msg_textbox])
        msg_textbox.submit(fn=submit_message, inputs=[msg_textbox, chatbot], outputs=[chatbot, msg_textbox])

        apply_button.click(fn=update_clients, inputs=[model_dropdown, headless_checkbox], outputs=[status_display])
        create_model_button.click(fn=handle_model_creation, inputs=[gguf_file_upload, new_model_name], outputs=[creation_status, model_dropdown, create_model_button])
    return app

# --- Запуск приложения ---
def start_gradio(app):
    app.launch(server_name="127.0.0.1", server_port=GRADIO_SERVER_PORT)

def on_closing():
    print("Window is closing, shutting down servers...")
    if ollama_process: ollama_process.terminate()
    print("Shutdown complete.")

def wait_for_ollama(max_retries=15):
    print("Waiting for Ollama server to be ready...")
    for i in range(max_retries):
        try:
            ollama_list()
            print("✅ Ollama server is ready.")
            return True
        except Exception:
            print(f"Attempt {i+1}/{max_retries}... server not ready.")
            time.sleep(2)
    print("❌ Ollama server did not start in time.")
    return False

if __name__ == '__main__':
    load_system_prompt()
    run_ollama_server()
    
    if wait_for_ollama():
        # Сначала получаем список моделей
        initial_models = get_ollama_models()
        if not initial_models:
            print("CRITICAL: No models found. Exiting.")
            if ollama_process: ollama_process.terminate()
            exit()
        
        app_settings["model"] = initial_models[0]
        
        # Инициализируем клиенты ПОСЛЕ того, как сервер точно готов
        update_clients(app_settings["model"], False)
        
        # Передаем список моделей в UI
        app = create_gradio_app(initial_models)
        
        gradio_thread = threading.Thread(target=start_gradio, args=(app,))
        gradio_thread.daemon = True
        gradio_thread.start()
        
        print(f"Gradio thread started. Waiting for UI at http://127.0.0.1:{GRADIO_SERVER_PORT}")
        time.sleep(2)

        window = webview.create_window('The Orchestrator', f'http://127.0.0.1:{GRADIO_SERVER_PORT}', width=1024, height=768)
        window.events.closing += on_closing
        webview.start()
    else:
        print("Could not start the application because Ollama server failed to respond.")
        if ollama_process: ollama_process.terminate()

    print("Application closed.")