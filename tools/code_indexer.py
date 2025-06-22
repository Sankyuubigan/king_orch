# tools/code_indexer.py - СКРИПТ ДЛЯ НАПОЛНЕНИЯ ВЕКТОРНОЙ "ПАМЯТИ"

import os
import requests
import json
from pathlib import Path
from typing import List, Dict, Any

# Убедитесь, что tree-sitter и нужные языки установлены
# pip install tree-sitter sentence-transformers tree-sitter-languages
try:
    from tree_sitter import Language, Parser
    from tree_sitter_languages import get_language
    from sentence_transformers import SentenceTransformer
except ImportError:
    print("="*80)
    print("!!! КРИТИЧЕСКАЯ ОШИБКА: НЕОБХОДИМЫЕ БИБЛИОТЕКИ НЕ НАЙДЕНЫ !!!")
    print("Пожалуйста, выполните в терминале команду:")
    print("pip install tree-sitter sentence-transformers tree-sitter-languages")
    print("="*80)
    exit(1)


# --- Конфигурация ---
PROJECT_ROOT = Path(__file__).parent.parent
SANDBOX_PATH = PROJECT_ROOT / "sandbox"
CHROMA_URL = "http://127.0.0.1:8011/add_texts"
COLLECTION_NAME = "project_code_memory"
# Модель, рекомендованная сообществом за хороший баланс качества и размера
EMBEDDING_MODEL = 'nomic-ai/nomic-embed-text-v1.5'

# --- Настройка Tree-sitter ---
# Мы будем извлекать функции и классы как отдельные документы
PYTHON_QUERY = """
(function_definition
  name: (identifier) @function.name) @function.definition

(class_definition
  name: (identifier) @class.name) @class.definition
"""

def get_python_chunks(file_path: Path, language: Language) -> List[Dict[str, Any]]:
    """Разбивает Python файл на осмысленные чанки (функции и классы) с помощью tree-sitter."""
    parser = Parser()
    parser.set_language(language)
    
    try:
        code_bytes = file_path.read_bytes()
        tree = parser.parse(code_bytes)
        
        chunks = []
        query = language.query(PYTHON_QUERY)
        captures = query.captures(tree.root_node)
        
        for node, name in captures:
            if name.endswith(".definition"):
                start_line = node.start_point[0] + 1
                end_line = node.end_point[0] + 1
                
                chunk_data = {
                    "text": node.text.decode('utf-8'),
                    "metadata": {
                        "source": str(file_path.relative_to(PROJECT_ROOT)).replace('\\', '/'),
                        "start_line": start_line,
                        "end_line": end_line,
                        "type": "function" if "function" in name else "class"
                    }
                }
                chunks.append(chunk_data)
        return chunks
    except Exception as e:
        print(f"  [!] Ошибка парсинга файла {file_path}: {e}")
        return []

def index_project():
    """Главная функция для индексации проекта."""
    print("="*80)
    print("НАЧАЛО ИНДЕКСАЦИИ ПРОЕКТА ДЛЯ СОЗДАНИЯ 'ПАМЯТИ' АГЕНТА")
    print("="*80)

    if not SANDBOX_PATH.exists() or not any(SANDBOX_PATH.iterdir()):
        print(f"[!] Директория '{SANDBOX_PATH}' не существует или пуста. Нечего индексировать.")
        return

    # 1. Загружаем модель для эмбеддингов
    print(f"[1/4] Загрузка embedding-модели '{EMBEDDING_MODEL}'... (может занять время при первом запуске)")
    try:
        model = SentenceTransformer(EMBEDDING_MODEL, trust_remote_code=True)
        print("  [+] Модель успешно загружена.")
    except Exception as e:
        print(f"  [!] Не удалось загрузить модель: {e}")
        return

    # 2. Собираем и парсим файлы
    print(f"[2/4] Поиск и парсинг .py файлов в '{SANDBOX_PATH}'...")
    all_chunks = []
    python_lang = get_language('python')
    
    for py_file in SANDBOX_PATH.rglob("*.py"):
        print(f"  - Обработка файла: {py_file.name}")
        chunks = get_python_chunks(py_file, python_lang)
        all_chunks.extend(chunks)
    
    if not all_chunks:
        print("  [!] Не найдено ни одного фрагмента кода для индексации.")
        return
    print(f"  [+] Найдено {len(all_chunks)} фрагментов кода (функций/классов).")

    # 3. Создаем эмбеддинги
    print("[3/4] Создание векторов (эмбеддингов) для фрагментов кода...")
    texts_to_embed = [chunk['text'] for chunk in all_chunks]
    embeddings = model.encode(texts_to_embed, show_progress_bar=True)
    print("  [+] Эмбеддинги успешно созданы.")

    # 4. Отправляем данные в ChromaDB
    print(f"[4/4] Отправка данных в ChromaDB (коллекция: '{COLLECTION_NAME}')...")
    
    # ChromaDB ожидает эмбеддинги как списки, а не numpy-массивы
    embeddings_list = [emb.tolist() for emb in embeddings]
    metadatas = [chunk['metadata'] for chunk in all_chunks]

    payload = {
        "collection_name": COLLECTION_NAME,
        "embeddings": embeddings_list,
        "documents": texts_to_embed, # `chroma-mcp` использует `documents`, а не `texts`
        "metadatas": metadatas
    }

    try:
        # `chroma-mcp` использует `upsert` для добавления/обновления
        response = requests.post("http://127.0.0.1:8011/upsert", json=payload, timeout=60)
        response.raise_for_status()
        print(f"  [+] Данные успешно отправлены в ChromaDB! Ответ сервера: {response.json()}")
    except requests.exceptions.RequestException as e:
        print(f"  [!] КРИТИЧЕСКАЯ ОШИБКА: Не удалось подключиться к серверу ChromaDB по адресу {CHROMA_URL}.")
        print("      Убедитесь, что все серверы запущены через main.py.")
        print(f"      Детали ошибки: {e}")
    
    print("="*80)
    print("ИНДЕКСАЦИЯ ЗАВЕРШЕНА")
    print("="*80)

if __name__ == "__main__":
    index_project()
