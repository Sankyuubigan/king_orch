# app.py

import streamlit as st
from llama_cpp import Llama
import os
import time
import logging
import io

# --- НАСТРОЙКА ЛОГИРОВАНИЯ ВНУТРИ STREAMLIT ---
# Создаем "поток" в памяти, куда будут писаться логи
log_stream = io.StringIO()

# Настраиваем логгер, чтобы он писал в наш поток
# Устанавливаем уровень DEBUG, чтобы ловить абсолютно все сообщения
logging.basicConfig(stream=log_stream, level=logging.DEBUG, format='%(asctime)s - %(levelname)s - %(message)s')

# --- ГЛАВНЫЕ НАСТРОЙКИ ---
MODELS_DIR = "D:/nn/models" # Используем прямые слэши

st.set_page_config(page_title="The Orchestrator", page_icon="🎭", layout="wide")
st.title("🎭 The Orchestrator")

# --- БЛОК ДЛЯ ОТОБРАЖЕНИЯ ЛОГОВ ---
log_expander = st.expander("Показать/скрыть подробные логи")
log_placeholder = log_expander.empty()

# --- ВСПОМОГАТЕЛЬНЫЕ ФУНКЦИИ ---

def find_gguf_models(directory):
    logging.info(f"Поиск моделей в папке: {directory}")
    if not os.path.isdir(directory):
        logging.error(f"Папка не найдена: {directory}")
        return []
    files = [f for f in os.listdir(directory) if f.endswith('.gguf')]
    logging.info(f"Найденные файлы: {files}")
    return files

@st.cache_resource
def load_model(model_filename):
    model_path = os.path.join(MODELS_DIR, model_filename)
    logging.info(f"Попытка загрузить модель из: {model_path}")
    
    if not os.path.exists(model_path):
        logging.error(f"Файл модели не найден: {model_path}")
        st.error(f"Файл модели не найден: {model_path}")
        st.stop()
    
    with st.spinner(f"Загрузка модели '{model_filename}'..."):
        llm = Llama(
            model_path=model_path,
            n_gpu_layers=0,
            n_ctx=4096,
            chat_format="disable",
            verbose=True # llama-cpp будет писать свои подробные логи
        )
    logging.info(f"Модель '{model_filename}' успешно загружена.")
    return llm

# --- ИНТЕРФЕЙС И ЛОГИКА ---

try:
    model_files = find_gguf_models(MODELS_DIR)

    if not model_files:
        st.error(f"Модели не найдены в папке: {MODELS_DIR}")
        st.stop()

    with st.sidebar:
        st.header("Настройки")
        selected_model_file = st.selectbox("Выберите модель:", model_files)
        if st.button("Начать новый диалог"):
            if "messages" in st.session_state:
                st.session_state.messages = []
            st.rerun()

    if selected_model_file:
        llm = load_model(selected_model_file)
    else:
        st.info("Пожалуйста, выберите модель в панели слева.")
        st.stop()

    if "messages" not in st.session_state:
        st.session_state.messages = []

    for message in st.session_state.messages:
        with st.chat_message(message["role"]):
            st.markdown(message["content"])

    if prompt := st.chat_input("Спросите что-нибудь..."):
        st.session_state.messages.append({"role": "user", "content": prompt})
        with st.chat_message("user"):
            st.markdown(prompt)

        with st.chat_message("assistant"):
            with st.spinner("Модель думает..."):
                full_prompt = "<|im_start|>system\nYou are a helpful AI assistant.<|im_end|>\n"
                for msg in st.session_state.messages:
                    full_prompt += f"<|im_start|>{msg['role']}\n{msg['content']}<|im_end|>\n"
                full_prompt += "<|im_start|>assistant\n"
                
                logging.info("Начинаю генерацию ответа...")
                output = llm(prompt=full_prompt, max_tokens=2048, stop=["<|im_end|>"])
                result = output['choices'][0]['text'].strip()
                logging.info("Генерация ответа завершена.")
                
                st.markdown(result)
                st.session_state.messages.append({"role": "assistant", "content": result})

except Exception as e:
    # Ловим абсолютно любую ошибку
    st.error(f"Произошла критическая ошибка: {e}")
    # Пишем полный traceback в наш логгер
    logging.error("Критическая ошибка в главном блоке:", exc_info=True)

finally:
    # В самом конце, обновляем текстовый блок с логами
    log_placeholder.code(log_stream.getvalue())