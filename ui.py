# ui.py - ВЕРСИЯ С УЛУЧШЕННЫМ ОТОБРАЖЕНИЕМ "ТЕЛЕВИЗОРА"

import threading
import tkinter as tk
from tkinter import ttk, scrolledtext, messagebox
import time
import requests
from io import BytesIO
from PIL import Image, ImageTk

from engine import OrchestratorEngine

SCREENSHOT_URL = "http://127.0.0.1:7777/screenshot"

class AppUI:
    def __init__(self, root_window, engine: OrchestratorEngine):
        self.root = root_window
        self.engine = engine
        self.browser_visible = False
        self.is_processing = False
        self.screenshot_thread = None
        self.stop_screenshot_thread = threading.Event()

        self.root.title("The Orchestrator v15.0 (Stealth Mode)")
        self.root.geometry("1700x800")
        
        self.create_widgets()
        self.populate_models_dropdown()
        self.update_token_count()

    def _handle_key_press(self, event):
        CONTROL_MASK = 0x0004
        if event.state & CONTROL_MASK:
            widget = event.widget
            if event.char == '\x03':
                try:
                    if widget.selection_get():
                        self.root.clipboard_clear()
                        self.root.clipboard_append(widget.selection_get())
                        return "break"
                except tk.TclError: pass
            elif event.char == '\x16':
                if widget == self.chat_input:
                    try:
                        widget.insert(tk.INSERT, self.root.clipboard_get())
                        return "break"
                    except tk.TclError: pass

    def create_widgets(self):
        self.main_pane = ttk.PanedWindow(self.root, orient=tk.HORIZONTAL)
        self.main_pane.pack(fill=tk.BOTH, expand=True)

        left_pane_container = ttk.Frame(self.main_pane, padding=5)
        self.main_pane.add(left_pane_container, weight=1)

        top_frame = ttk.Frame(left_pane_container)
        top_frame.pack(side=tk.TOP, fill=tk.X, pady=5)
        self.model_combo = ttk.Combobox(top_frame, state="readonly", width=40)
        self.model_combo.pack(side=tk.LEFT, pady=5, padx=5, fill=tk.X, expand=True)
        self.load_button = ttk.Button(top_frame, text="Загрузить", command=self.start_load_task)
        self.load_button.pack(side=tk.LEFT, pady=5, padx=5)
        self.unload_button = ttk.Button(top_frame, text="Выгрузить", command=self.start_unload_task)
        self.unload_button.pack(side=tk.LEFT, pady=5, padx=5)
        self.browser_toggle_button = ttk.Button(top_frame, text="Показать браузер", command=self.toggle_browser_visibility)
        self.browser_toggle_button.pack(side=tk.LEFT, pady=5, padx=5)

        notebook = ttk.Notebook(left_pane_container)
        notebook.pack(fill=tk.BOTH, expand=True, side=tk.TOP)
        chat_tab = ttk.Frame(notebook); log_tab = ttk.Frame(notebook)
        notebook.add(chat_tab, text='Чат'); notebook.add(log_tab, text='Логи')
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
        self.chat_input.bind("<KeyPress>", self._handle_key_press)
        self.chat_area.bind("<KeyPress>", self._handle_key_press)
        self.log_area.bind("<KeyPress>", self._handle_key_press)
        status_bar = ttk.Frame(left_pane_container, padding=(5, 5))
        status_bar.pack(side=tk.BOTTOM, fill=tk.X)
        self.token_count_label = ttk.Label(status_bar, text="Токены: 0")
        self.token_count_label.pack(side=tk.LEFT)
        self.progress_bar = ttk.Progressbar(status_bar, mode='indeterminate', length=150)
        self.progress_bar.pack(side=tk.RIGHT)

        self.browser_container = ttk.Frame(self.main_pane, width=600)
        # <<< ИЗМЕНЕНИЕ: Добавляем текст по умолчанию >>>
        self.screenshot_label = ttk.Label(self.browser_container, text="Ожидание скриншота...", anchor="center")
        self.screenshot_label.pack(fill=tk.BOTH, expand=True)

    def toggle_browser_visibility(self):
        if self.browser_visible:
            self.stop_screenshot_thread.set()
            self.main_pane.forget(self.browser_container)
            self.browser_toggle_button.config(text="Показать браузер")
        else:
            self.main_pane.add(self.browser_container, weight=2)
            self.browser_toggle_button.config(text="Скрыть браузер")
            self.stop_screenshot_thread.clear()
            self.screenshot_thread = threading.Thread(target=self._screenshot_loop, daemon=True)
            self.screenshot_thread.start()
        self.browser_visible = not self.browser_visible

    def _screenshot_loop(self):
        while not self.stop_screenshot_thread.is_set():
            try:
                response = requests.get(SCREENSHOT_URL, timeout=5)
                response.raise_for_status()
                image_data = Image.open(BytesIO(response.content))
                
                container_width = self.screenshot_label.winfo_width()
                container_height = self.screenshot_label.winfo_height()
                if container_width > 1 and container_height > 1:
                    image_data.thumbnail((container_width, container_height), Image.Resampling.LANCZOS)

                photo = ImageTk.PhotoImage(image_data)
                # Убираем текст и ставим изображение
                self.root.after(0, self.screenshot_label.config, {"image": photo, "text": ""})
                self.screenshot_label.image = photo
            except requests.exceptions.RequestException:
                self.root.after(0, self.screenshot_label.config, {"image": "", "text": "Не удалось получить скриншот..."})
                time.sleep(1)
            except Exception as e:
                self.log_to_widget(f"[Screenshot] Ошибка: {e}")
            
            time.sleep(3)

    def _insert_chat_message(self, message, tag=None):
        self.chat_area.config(state=tk.NORMAL)
        self.chat_area.insert(tk.END, message + "\n\n")
        self.chat_area.see(tk.END)
        self.chat_area.config(state=tk.DISABLED)

    def start_chat_task(self, event=None):
        prompt = self.chat_input.get()
        if not prompt or self.is_processing: return
        self.chat_input.delete(0, tk.END)
        self._insert_chat_message(f"Вы: {prompt}")
        self.set_ui_busy(True)
        threading.Thread(target=self._get_engine_decision, args=(prompt,), daemon=True).start()

    def _get_engine_decision(self, prompt):
        response = self.engine.get_response(prompt)
        self.root.after(0, self._process_engine_decision, response)

    def _process_engine_decision(self, response):
        status = response.get("status")
        if status == "tool_call":
            user_message = response.get("user_message", "Начинаю поиск...")
            self._insert_chat_message(f"Модель: {user_message}")
            threading.Thread(target=self._execute_tool_and_get_final_answer, args=(response,), daemon=True).start()
        else:
            self._finalize_chat_response(response["content"])

    def _execute_tool_and_get_final_answer(self, decision_response):
        final_response_obj = self.engine.execute_tool_and_continue(
            decision_response["tool_data"], 
            decision_response["full_model_response"]
        )
        self.root.after(0, self._finalize_chat_response, final_response_obj["content"])

    def _finalize_chat_response(self, response_text):
        final_response = response_text.strip() if response_text and response_text.strip() else "[Модель не вернула ответ]"
        self._insert_chat_message(f"Модель: {final_response}")
        self.set_ui_busy(False)
        self.update_token_count()

    def set_ui_busy(self, is_busy):
        self.is_processing = is_busy
        state = tk.DISABLED if is_busy else tk.NORMAL
        self.load_button.config(state=state)
        self.unload_button.config(state=state)
        self.model_combo.config(state=tk.DISABLED if is_busy else "readonly")
        self.send_button.config(state=state)
        self.chat_input.config(state=state)
        if is_busy: self.progress_bar.start()
        else: self.progress_bar.stop()

    def _clear_chat_display(self):
        self.chat_area.config(state=tk.NORMAL)
        self.chat_area.delete('1.0', tk.END)
        self.chat_area.config(state=tk.DISABLED)

    def update_token_count(self):
        count = self.engine.get_current_token_count()
        self.token_count_label.config(text=f"Токены: {count}")

    def populate_models_dropdown(self):
        models = self.engine.get_available_models()
        self.model_combo['values'] = models if models else ["Модели не найдены!"]
        if models: self.model_combo.current(0)

    def start_load_task(self):
        if self.is_processing: return
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
        if self.is_processing: return
        self.set_ui_busy(True)
        threading.Thread(target=self._unload_model_thread_target, daemon=True).start()

    def _unload_model_thread_target(self):
        self.engine.unload_model()
        self.root.after(0, self._clear_chat_display)
        self.root.after(0, self.set_ui_busy, False)
        self.root.after(0, self.update_token_count)
        
    def log_to_widget(self, message):
        if self.root.winfo_exists():
            self.root.after(0, self._insert_log_message, message)

    def _insert_log_message(self, message):
        self.log_area.config(state=tk.NORMAL)
        self.log_area.insert(tk.END, message + "\n")
        self.log_area.see(tk.END)
        self.log_area.config(state=tk.DISABLED)