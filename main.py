# main.py - Интерфейс, который правильно работает с движком

import threading
import tkinter as tk
from tkinter import ttk, scrolledtext, messagebox
from engine import OrchestratorEngine # <-- Импортируем новый движок

class AppUI:
    def __init__(self, root_window):
        self.root = root_window
        self.root.title("The Orchestrator (Архитектура v3.0 - Финальная)")
        self.root.geometry("1200x800")
        self.engine = OrchestratorEngine(log_callback=self.log_to_widget)
        self.create_widgets()
        self.log_to_widget("[UI] Интерфейс инициализирован.")
        self.populate_models_dropdown()

    def log_to_widget(self, message):
        self.root.after(0, self._insert_log_message, message)

    def _insert_log_message(self, message):
        self.log_area.config(state=tk.NORMAL)
        self.log_area.insert(tk.END, message + "\n")
        self.log_area.see(tk.END)
        self.log_area.config(state=tk.DISABLED)

    def create_widgets(self):
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
        notebook = ttk.Notebook(right_frame)
        notebook.pack(fill=tk.BOTH, expand=True)
        chat_tab = ttk.Frame(notebook)
        log_tab = ttk.Frame(notebook)
        notebook.add(chat_tab, text='Чат')
        notebook.add(log_tab, text='Логи')
        self.log_area = scrolledtext.ScrolledText(log_tab, wrap=tk.WORD, state=tk.DISABLED)
        self.log_area.pack(fill=tk.BOTH, expand=True)
        self.chat_area = scrolledtext.ScrolledText(chat_tab, wrap=tk.WORD, state=tk.DISABLED)
        self.chat_area.pack(fill=tk.BOTH, expand=True)
        input_frame = ttk.Frame(chat_tab)
        input_frame.pack(fill=tk.X, pady=5)
        self.chat_input = ttk.Entry(input_frame)
        self.chat_input.pack(side=tk.LEFT, fill=tk.X, expand=True, padx=(0, 5))
        self.chat_input.bind("<Return>", self.start_chat_task)
        self.send_button = ttk.Button(input_frame, text="Отправить", command=self.start_chat_task)
        self.send_button.pack(side=tk.RIGHT)

    def populate_models_dropdown(self):
        models = self.engine.get_available_models()
        self.model_combo['values'] = models if models else ["Модели не найдены!"]
        self.model_combo.current(0)

    def set_ui_busy(self, is_busy):
        """Блокирует или разблокирует UI во время операций."""
        state = tk.DISABLED if is_busy else tk.NORMAL
        self.load_button.config(state=state)
        self.unload_button.config(state=state)
        self.model_combo.config(state=tk.DISABLED if is_busy else "readonly")
        self.send_button.config(state=state)
        if is_busy:
            self.progress_bar.start()
        else:
            self.progress_bar.stop()

    def start_load_task(self):
        selected_model = self.model_combo.get()
        if "Выберите" in selected_model or "найдены" in selected_model:
            messagebox.showwarning("Внимание", "Пожалуйста, выберите корректную модель.")
            return
        self.set_ui_busy(True)
        threading.Thread(target=self._load_model_thread_target, args=(selected_model,), daemon=True).start()

    def _load_model_thread_target(self, model_name):
        success = self.engine.load_model(model_name)
        if success:
            # Очищаем чат при успешной загрузке новой модели
            self.root.after(0, self._clear_chat_display)
        self.root.after(0, self.set_ui_busy, False)

    def start_unload_task(self):
        self.set_ui_busy(True)
        threading.Thread(target=self._unload_model_thread_target, daemon=True).start()

    def _unload_model_thread_target(self):
        self.engine.unload_model()
        self.root.after(0, self._clear_chat_display)
        self.root.after(0, self.set_ui_busy, False)

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

    def _insert_chat_message(self, message):
        self.chat_area.config(state=tk.NORMAL)
        self.chat_area.insert(tk.END, message + "\n\n")
        self.chat_area.see(tk.END)
        self.chat_area.config(state=tk.DISABLED)

    def _clear_chat_display(self):
        self.chat_area.config(state=tk.NORMAL)
        self.chat_area.delete('1.0', tk.END)
        self.chat_area.config(state=tk.DISABLED)

if __name__ == "__main__":
    main_window = tk.Tk()
    app = AppUI(main_window)
    main_window.mainloop()