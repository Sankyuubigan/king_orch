import os
import signal
import subprocess
import threading
import time
import gradio as gr
import webview
import json
import shutil
from ollama import Client as OllamaChatClient
from ollama import list as ollama_list

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
    try:
        models = ollama_list()['models']
        return [model['name'] for model in models]
    except Exception:
        return [DEFAULT_MODEL]

def run_ollama_server():
    global ollama_process
    print("Starting Ollama Server...")
    ollama_process = subprocess.Popen([OLLAMA_PATH, "serve"], creationflags=subprocess.CREATE_NO_WINDOW)
    print(f"Ollama Server started with PID: {ollama_process.pid}")

def update_clients(model_name, show_browser):
    global agent, ollama_chat_client, app_settings
    print(f"Updating settings: Model -> {model_name}, Show Browser -> {show_browser}")
    app_settings["model"] = model_name
    app_settings["headless"] = not show_browser
    try:
        agent = Stagehand(model=f"ollama/{model_name}", browser_options={"headless": app_settings["headless"]})
        ollama_chat_client = OllamaChatClient(host='http://127.0.0.1:11434')
        status_message = f"✅ Настройки применены! Активная модель: `{model_name}`."
        print(status_message)
        return status_message
    except Exception as e:
        return f"❌ Ошибка: {e}"

def handle_model_creation(file_obj, model_name):
    """Создает новую модель в Ollama из GGUF файла."""
    if not file_obj or not model_name:
        return "❌ Ошибка: Выберите файл и укажите имя для модели.", gr.Dropdown(choices=get_ollama_models(), value=app_settings["model"])
    
    model_name = model_name.strip().lower().replace(" ", "-")
    gguf_path = file_obj.name
    modelfile_path = f"./{model_name}.modelfile"
    
    try:
        print(f"Creating Modelfile for {model_name} at {modelfile_path}")
        with open(modelfile_path, 'w') as f:
            f.write(f'FROM "{os.path.abspath(gguf_path)}"')
        
        print(f"Running: ollama create {model_name} -f {modelfile_path}")
        
        # Запускаем процесс и ждем завершения
        result = subprocess.run(
            ["ollama", "create", model_name, "-f", modelfile_path],
            capture_output=True, text=True, check=True
        )
        print(result.stdout)
        os.remove(modelfile_path) # Удаляем временный Modelfile
        
        # Обновляем список моделей в выпадающем меню
        updated_models = get_ollama_models()
        return f"✅ Модель '{model_name}' успешно создана!", gr.Dropdown(choices=updated_models, value=model_name)

    except subprocess.CalledProcessError as e:
        print(f"Ollama create error: {e.stderr}")
        return f"❌ Ошибка Ollama: {e.stderr}", gr.Dropdown(choices=get_ollama_models(), value=app_settings["model"])
    except Exception as e:
        print(f"An error occurred: {e}")
        return f"❌ Неизвестная ошибка: {e}", gr.Dropdown(choices=get_ollama_models(), value=app_settings["model"])


# --- Основная логика чата ---
def chat_function(message, history):
    print(f"User message: {message}")
    
    # ИСПРАВЛЕНО: Правильная обработка истории
    conversation = [{'role': 'system', 'content': SYSTEM_PROMPT}]
    for user_msg, assistant_msg in history:
        conversation.append({'role': 'user', 'content': user_msg})
        clean_assistant_msg = assistant_msg.replace("🔎 ", "", 1)
        if not clean_assistant_msg.startswith("🤖") and not clean_assistant_msg.startswith("✅"):
            conversation.append({'role': 'assistant', 'content': clean_assistant_msg})
    conversation.append({'role': 'user', 'content': message})

    try:
        response = ollama_chat_client.chat(model=app_settings["model"], messages=conversation)
        model_response_content = response['message']['content']
        print(f"Model initial response: {model_response_content}")

        try:
            tool_call = json.loads(model_response_content)
            if tool_call.get("tool") == "stagehand_search":
                # ... (логика Stagehand осталась без изменений)
                query = tool_call.get("query")
                yield "🤖 *Модель решила использовать Stagehand. Выполняю поиск...*"
                stagehand_result = agent.invoke(goal=query, url="https://www.google.com")
                search_answer = stagehand_result.get('answer', 'Не удалось извлечь ответ.')
                yield "✅ *Поиск завершен. Формулирую итоговый ответ...*"
                final_prompt = conversation + [{'role': 'assistant', 'content': model_response_content}, {'role': 'system', 'content': f"Результат поиска: {search_answer}\nНа основе этого дай ответ."}]
                final_response_stream = ollama_chat_client.chat(model=app_settings["model"], messages=final_prompt, stream=True)
                full_response = ""
                for chunk in final_response_stream:
                    full_response += chunk['message']['content']
                    yield f"🔎 {full_response}"
                return
        except (json.JSONDecodeError, TypeError, KeyError):
            yield model_response_content
    except Exception as e:
        yield f"Произошла непредвиденная ошибка: {e}"

# --- Сборка интерфейса Gradio ---
with gr.Blocks(theme=gr.themes.Soft(), title="The Orchestrator") as app:
    gr.Markdown("# The Orchestrator (Agent Edition)")
    
    with gr.Tabs():
        with gr.TabItem("Чат"):
            with gr.Row():
                with gr.Column(scale=1):
                    gr.Markdown("### Настройки чата")
                    model_dropdown = gr.Dropdown(label="Активная модель", choices=get_ollama_models(), value=DEFAULT_MODEL)
                    headless_checkbox = gr.Checkbox(label="Показывать браузер Stagehand", value=False)
                    apply_button = gr.Button("Применить настройки")
                    status_display = gr.Markdown("")
                with gr.Column(scale=3):
                    gr.ChatInterface(fn=chat_function, chatbot=gr.Chatbot(height=600, label="Чат"), textbox=gr.Textbox(placeholder="Спроси меня о чем-нибудь...", container=False, scale=7))
        
        with gr.TabItem("Управление моделями"):
            gr.Markdown("### Создать новую модель из GGUF файла")
            with gr.Row():
                gguf_file_upload = gr.File(label="Выберите .gguf файл", file_types=['.gguf'])
                new_model_name = gr.Textbox(label="Придумайте короткое имя для модели (например, 'dolphin-cool')")
            create_model_button = gr.Button("Создать модель")
            creation_status = gr.Markdown("")

    # Привязываем функции к кнопкам
    apply_button.click(fn=update_clients, inputs=[model_dropdown, headless_checkbox], outputs=[status_display])
    create_model_button.click(
        fn=handle_model_creation, 
        inputs=[gguf_file_upload, new_model_name], 
        outputs=[creation_status, model_dropdown] # Обновляем и статус, и список моделей
    )

# --- Запуск приложения ---
def start_gradio():
    app.launch(server_name="127.0.0.1", server_port=GRADIO_SERVER_PORT)

def on_closing():
    print("Window is closing, shutting down servers...")
    if ollama_process: ollama_process.terminate()
    print("Shutdown complete.")

if __name__ == '__main__':
    load_system_prompt()
    run_ollama_server()
    time.sleep(3) # Даем Ollama время на запуск перед инициализацией
    update_clients(DEFAULT_MODEL, False)

    gradio_thread = threading.Thread(target=start_gradio)
    gradio_thread.daemon = True
    gradio_thread.start()
    
    time.sleep(3)

    window = webview.create_window('The Orchestrator', f'http://127.0.0.1:{GRADIO_SERVER_PORT}', width=1024, height=768)
    window.events.closing += on_closing
    webview.start()

    print("Application closed.")