import tkinter as tk
from tkinter import ttk, scrolledtext, messagebox, simpledialog
from queue import Queue
import sys
import os
import base64
from io import BytesIO
import requests
from PIL import Image, ImageTk

from utils.clipboard_fortress import handle_keypress_event
from settings_window import SettingsWindow

FEEDBACK_URL_GET = "http://127.0.0.1:7787/get_question"
FEEDBACK_URL_POST = "http://127.0.0.1:7787/provide_answer"
LISTENING_BG_COLOR = "#E0F7FF"

class AppUI:
    def __init__(self, root_window, task_queue: Queue):
        self.root = root_window
        self.task_queue = task_queue
        self.is_processing = False
        self.voice_controller = None
        self.browser_photo_image = None
        self.last_input_was_voice = False
        self.default_bg_color = None

        self.root.title("Universal Orchestrator (v4.1 - Resilient)")
        self.root.geometry("1700x800")
        
        if sys.platform == "win32" and os.path.exists("logo.ico"):
            try: self.root.iconbitmap("logo.ico")
            except tk.TclError: print("[UI] [WARNING] Не удалось загрузить иконку 'logo.ico'.")
        
        self.create_widgets()
        self.default_bg_color = self.chat_input.cget("background")
        self.root.after(3000, self._feedback_poll_loop)

    def _open_settings_window(self):
        # ИСПРАВЛЕНИЕ: Теперь можно открыть настройки, даже если движок не готов,
        # но он будет передан как None, и окно настроек это обработает.
        SettingsWindow(self.root, self.voice_controller)

    def _insert_chat_message(self, message, author_prefix):
        # ... (код без изменений)
        self.chat_area.config(state=tk.NORMAL)
        self.chat_area.insert(tk.END, f"{author_prefix}: {message}\n\n")
        self.chat_area.see(tk.END)
        self.chat_area.config(state=tk.DISABLED)

    def submit_task_from_input(self, event=None):
        # ... (код без изменений)
        if self.is_processing: return
        prompt = self.chat_input.get()
        if not prompt: return
        
        self.chat_input.delete(0, tk.END)
        author = "Вы (🎤)" if self.last_input_was_voice else "Вы"
        self._insert_chat_message(prompt, author_prefix=author)
        self.task_queue.put(prompt)
        self.last_input_was_voice = False

    def create_widgets(self):
        # ... (код без изменений)
        self.main_pane = ttk.PanedWindow(self.root, orient=tk.HORIZONTAL)
        self.main_pane.pack(fill=tk.BOTH, expand=True)
        
        left_pane_container = ttk.Frame(self.main_pane, padding=5)
        self.main_pane.add(left_pane_container, weight=1)
        
        right_pane_container = ttk.LabelFrame(self.main_pane, text="Обзор браузера", padding=5)
        self.main_pane.add(right_pane_container, weight=2)
        
        self.browser_view_label = ttk.Label(right_pane_container, anchor=tk.CENTER)
        self.browser_view_label.pack(fill=tk.BOTH, expand=True)

        top_frame = ttk.Frame(left_pane_container)
        top_frame.pack(side=tk.TOP, fill=tk.X, pady=5)
        
        self.info_label = ttk.Label(top_frame, text="Инициализация, пожалуйста, подождите...")
        self.info_label.pack(side=tk.LEFT, pady=5, padx=5, fill=tk.X, expand=True)
        
        button_panel = ttk.Frame(top_frame)
        button_panel.pack(side=tk.RIGHT, padx=5)
        self.settings_button = ttk.Button(button_panel, text="⚙️ Настройки", command=self._open_settings_window)
        self.settings_button.pack(side=tk.RIGHT)
        
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
        self.send_button = ttk.Button(input_frame, text="Отправить", command=self.submit_task_from_input)
        self.send_button.pack(side=tk.RIGHT)
        
        self.chat_input.bind("<Return>", self.submit_task_from_input)
        for widget in [self.chat_input, self.log_area, self.chat_area]:
            widget.bind("<KeyPress>", self._pass_to_fortress)
        
        status_bar = ttk.Frame(left_pane_container, padding=(5, 5))
        status_bar.pack(side=tk.BOTTOM, fill=tk.X)
        self.progress_bar = ttk.Progressbar(status_bar, mode='indeterminate', length=150)
        self.progress_bar.pack(side=tk.RIGHT)
        
        self.set_ui_busy(True, "Инициализация фоновых сервисов...")
        self.chat_input.focus_set()

    def set_voice_controller(self, controller):
        self.voice_controller = controller

    def set_listening_status(self, is_listening: bool):
        # ... (код без изменений)
        if not self.root.winfo_exists(): return
        if is_listening:
            self.root.after(0, self.info_label.config, {"text": "Слушаю вашу команду..."})
            self.root.after(0, self.chat_input.config, {"background": LISTENING_BG_COLOR})
        else:
            self.root.after(0, self.info_label.config, {"text": "Готов к работе. Ожидание задач..."})
            self.root.after(0, self.chat_input.config, {"background": self.default_bg_color})

    def show_partial_transcription(self, text: str):
        # ... (код без изменений)
        if not self.root.winfo_exists(): return
        self.root.after(0, self._update_input_text, text)

    def _update_input_text(self, text):
        # ... (код без изменений)
        self.chat_input.delete(0, tk.END)
        self.chat_input.insert(0, text)

    def set_ui_busy(self, is_busy, status_text=None):
        if not self.root.winfo_exists(): return
        self.is_processing = is_busy
        
        if status_text: self.set_info_label(status_text)
        elif not is_busy: self.set_info_label("Готов к работе. Ожидание задач...")

        new_state = tk.DISABLED if is_busy else tk.NORMAL
        self.send_button.config(state=new_state)
        self.chat_input.config(state=new_state)
        
        if is_busy: self.progress_bar.start(10)
        else: self.progress_bar.stop()

    def unlock_settings_button(self):
        """Принудительно разблокирует кнопку настроек в случае критической ошибки."""
        if self.root.winfo_exists():
            self.root.after(0, self.settings_button.config, {"state": tk.NORMAL})

    def set_info_label(self, text: str):
        # ... (код без изменений)
        if self.root.winfo_exists():
            self.root.after(0, self.info_label.config, {"text": text})

    def update_chat_with_final_result(self, final_result_text: str):
        # ... (код без изменений)
        if self.root.winfo_exists():
            self.root.after(0, lambda: self._insert_chat_message(final_result_text, author_prefix="Ассистент"))

    def log_to_widget(self, message):
        # ... (код без изменений)
        if self.root.winfo_exists():
            self.root.after(0, self._insert_log_message, message)

    def _insert_log_message(self, message):
        # ... (код без изменений)
        self.log_area.config(state=tk.NORMAL)
        self.log_area.insert(tk.END, str(message) + "\n")
        self.log_area.see(tk.END)
        self.log_area.config(state=tk.DISABLED)

    def _feedback_poll_loop(self):
        # ... (код без изменений)
        try:
            response = requests.get(FEEDBACK_URL_GET, timeout=0.5)
            if response.status_code == 200 and (question := response.json().get("question")):
                self.log_to_widget(f"[Feedback] Получен вопрос для пользователя: {question}")
                self.root.after(0, self._ask_user_for_feedback, question)
        except requests.exceptions.RequestException: pass
        finally:
            if self.root.winfo_exists(): self.root.after(3000, self._feedback_poll_loop)

    def _ask_user_for_feedback(self, question: str):
        # ... (код без изменений)
        answer = simpledialog.askstring("Вопрос от Агента", question, parent=self.root)
        if answer is None: answer = "Пользователь отменил ввод."
        try:
            requests.post(FEEDBACK_URL_POST, json={"answer": answer}, timeout=5)
            self.log_to_widget(f"[Feedback] Ответ '{answer}' отправлен агенту.")
        except requests.exceptions.RequestException as e: self.log_to_widget(f"[Feedback] [ERROR] Не удалось отправить ответ: {e}")
        
    def _pass_to_fortress(self, event): return handle_keypress_event(event, self.log_to_widget, self.chat_input)
        
    def _update_browser_view(self, b64_string: str):
        # ... (код без изменений)
        if not b64_string: return
        try:
            image_bytes = base64.b64decode(b64_string)
            image = Image.open(BytesIO(image_bytes))
            panel_width = self.browser_view_label.winfo_width()
            if panel_width < 2: 
                self.root.after(100, self._update_browser_view, b64_string)
                return
            image.thumbnail((panel_width - 10, self.browser_view_label.winfo_height() - 10), Image.Resampling.LANCZOS)
            self.browser_photo_image = ImageTk.PhotoImage(image)
            self.browser_view_label.config(image=self.browser_photo_image)
        except Exception as e: self.log_to_widget(f"[UI] [ERROR] Не удалось отобразить скриншот: {e}")