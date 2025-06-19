# ui.py - ИСПРАВЛЕННАЯ ВЕРСИЯ С НИЗКОУРОВНЕВЫМ ОБРАБОТЧИКОМ ХОТКЕЕВ

import threading
import tkinter as tk
from tkinter import ttk, scrolledtext, messagebox
from engine import OrchestratorEngine

class AppUI:
    def __init__(self, root_window, engine: OrchestratorEngine):
        self.root = root_window
        self.engine = engine
        self.root.title("The Orchestrator v6.2 (Прямая обработка хоткеев)")
        self.root.geometry("1200x800")
        
        self.create_widgets()
        self.engine.log("[UI] Интерфейс инициализирован.")
        self.populate_models_dropdown()
        self.update_token_count()

    # <<< НОВЫЙ МЕТОД, РЕАЛИЗОВАННЫЙ СТРОГО ПО ВАШЕМУ ТРЕБОВАНИЮ >>>
    def _handle_key_press(self, event):
        """Анализирует низкоуровневые атрибуты событий для Ctrl+C и Ctrl+V."""
        CONTROL_MASK = 0x0004
        
        # Проверяем, зажата ли клавиша Control
        if event.state & CONTROL_MASK:
            widget = event.widget
            
            # Обработка Ctrl+C (char code \x03)
            if event.char == '\x03':
                try:
                    if widget.selection_get():
                        self.root.clipboard_clear()
                        self.root.clipboard_append(widget.selection_get())
                        return "break" # Прерываем дальнейшую обработку события
                except tk.TclError:
                    pass # Нет выделения

            # Обработка Ctrl+V (char code \x16)
            elif event.char == '\x16':
                # Вставка работает только в поле ввода
                if widget == self.chat_input:
                    try:
                        widget.insert(tk.INSERT, self.root.clipboard_get())
                        return "break" # Прерываем дальнейшую обработку события
                    except tk.TclError:
                        pass # Буфер обмена пуст

    def log_to_widget(self, message):
        self.root.after(0, self._insert_log_message, message)

    def _insert_log_message(self, message):
        self.log_area.config(state=tk.NORMAL)
        self.log_area.insert(tk.END, message + "\n")
        self.log_area.see(tk.END)
        self.log_area.config(state=tk.DISABLED)

    def create_widgets(self):
        # ... (левая панель без изменений) ...
        left_frame = ttk.Frame(self.root, padding="10")
        left_frame.pack(side=tk.LEFT, fill=tk.Y, padx=5, pady=5)
        ttk.Label(left_frame, text="ПУЛЬТ УПРАВЛЕНИЯ", font=("Segoe UI", 12, "bold")).pack(pady=10)
        self.model_combo = ttk.Combobox(left_frame, state="readonly", width=40)
        self.model_combo.pack(pady=5)
        self.model_combo.set("Выберите модель...")
        self.load_button = ttk.Button(left_frame, text="Загрузить модель", command=self.start_load_task)
        self.load_button.pack(pady=5, fill=tk.X)
        self.unload_button = ttk.Button(left_frame, text="Выгрузить модель", command=self.start_unload_task)
        self.unload_button.pack(pady=5, fill=tk.X)
        self.progress_bar = ttk.Progressbar(left_frame, mode='indeterminate')
        self.progress_bar.pack(pady=10, fill=tk.X)
        
        right_frame = ttk.Frame(self.root, padding="10")
        right_frame.pack(side=tk.RIGHT, fill=tk.BOTH, expand=True)
        
        status_bar = ttk.Frame(right_frame, padding=(0, 5))
        status_bar.pack(side=tk.BOTTOM, fill=tk.X)
        self.token_count_label = ttk.Label(status_bar, text="Токены в чате: 0")
        self.token_count_label.pack(side=tk.LEFT)

        notebook = ttk.Notebook(right_frame)
        notebook.pack(fill=tk.BOTH, expand=True, side=tk.TOP)
        
        chat_tab = ttk.Frame(notebook)
        log_tab = ttk.Frame(notebook)
        notebook.add(chat_tab, text='Чат')
        notebook.add(log_tab, text='Логи')
        
        # Убираем state=DISABLED, чтобы виджеты могли обрабатывать события и выделение
        self.log_area = scrolledtext.ScrolledText(log_tab, wrap=tk.WORD)
        self.log_area.pack(fill=tk.BOTH, expand=True)
        
        self.chat_area = scrolledtext.ScrolledText(chat_tab, wrap=tk.WORD)
        self.chat_area.pack(fill=tk.BOTH, expand=True)
        
        input_frame = ttk.Frame(chat_tab)
        input_frame.pack(fill=tk.X, pady=5)
        
        self.chat_input = ttk.Entry(input_frame)
        self.chat_input.pack(side=tk.LEFT, fill=tk.X, expand=True, padx=(0, 5))
        self.chat_input.bind("<Return>", self.start_chat_task)
        
        self.send_button = ttk.Button(input_frame, text="Отправить", command=self.start_chat_task)
        self.send_button.pack(side=tk.RIGHT)

        # <<< ПРИВЯЗЫВАЕМ НАШ НИЗКОУРОВНЕВЫЙ ОБРАБОТЧИК К ВИДЖЕТАМ >>>
        self.chat_input.bind("<KeyPress>", self._handle_key_press)
        self.chat_area.bind("<KeyPress>", self._handle_key_press)
        self.log_area.bind("<KeyPress>", self._handle_key_press)

    def _insert_chat_message(self, message):
        self.chat_area.config(state=tk.NORMAL)
        self.chat_area.insert(tk.END, message + "\n\n")
        self.chat_area.see(tk.END)
        self.chat_area.config(state=tk.DISABLED)

    def _clear_chat_display(self):
        self.chat_area.config(state=tk.NORMAL)
        self.chat_area.delete('1.0', tk.END)
        self.chat_area.config(state=tk.DISABLED)

    # ... (остальные методы без изменений) ...
    def update_token_count(self):
        count = self.engine.get_current_token_count()
        self.token_count_label.config(text=f"Токены в чате: {count}")

    def populate_models_dropdown(self):
        models = self.engine.get_available_models()
        self.model_combo['values'] = models if models else ["Модели не найдены!"]
        if models: self.model_combo.current(0)

    def set_ui_busy(self, is_busy):
        state = tk.DISABLED if is_busy else tk.NORMAL
        self.load_button.config(state=state)
        self.unload_button.config(state=state)
        self.model_combo.config(state=tk.DISABLED if is_busy else "readonly")
        self.send_button.config(state=state)
        if is_busy: self.progress_bar.start()
        else: self.progress_bar.stop()

    def start_load_task(self):
        selected_model = self.model_combo.get()
        if "Выберите" in selected_model or "найдены" in selected_model:
            messagebox.showwarning("Внимание", "Пожалуйста, выберите корректную модель.")
            return
        self.set_ui_busy(True)
        threading.Thread(target=self._load_model_thread_target, args=(selected_model,), daemon=True).start()

    def _load_model_thread_target(self, model_name):
        success = self.engine.load_model(model_name)
        if success: self.root.after(0, self._clear_chat_display)
        self.root.after(0, self.set_ui_busy, False)
        self.root.after(0, self.update_token_count)

    def start_unload_task(self):
        self.set_ui_busy(True)
        threading.Thread(target=self._unload_model_thread_target, daemon=True).start()

    def _unload_model_thread_target(self):
        self.engine.unload_model()
        self.root.after(0, self._clear_chat_display)
        self.root.after(0, self.set_ui_busy, False)
        self.root.after(0, self.update_token_count)

    def start_chat_task(self, event=None):
        prompt = self.chat_input.get()
        if not prompt: return
        self.chat_input.delete(0, tk.END)
        self._insert_chat_message(f"Вы: {prompt}")
        self.set_ui_busy(True)
        threading.Thread(target=self._get_engine_response, args=(prompt,), daemon=True).start()

    def _get_engine_response(self, prompt):
        response = self.engine.get_response(prompt)
        self.root.after(0, self._finalize_chat_response, response)

    def _finalize_chat_response(self, response):
        self._insert_chat_message(f"Модель: {response}")
        self.set_ui_busy(False)
        self.root.after(0, self.update_token_count)