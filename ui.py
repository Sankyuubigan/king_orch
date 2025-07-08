import tkinter as tk
from tkinter import ttk, scrolledtext, messagebox, simpledialog
from queue import Queue, Empty
import sys
import os
import base64
from io import BytesIO
import requests
from PIL import Image, ImageTk

from utils.clipboard_fortress import handle_keypress_event
from settings_window import SettingsWindow
from utils.text_utils import extract_thoughts

class CollapsiblePane(ttk.Frame):
    def __init__(self, parent, text="", body=""):
        super().__init__(parent, style="Card.TFrame", padding=5)
        self.parent = parent
        
        self.header_frame = ttk.Frame(self, style="Card.TFrame")
        self.header_frame.pack(fill="x", expand=True)

        self.arrow_label = ttk.Label(self.header_frame, text="▶ ", style="Card.TLabel")
        self.arrow_label.pack(side="left")
        
        title_label = ttk.Label(self.header_frame, text=text, style="Card.TLabel", font=("TkDefaultFont", 9, "bold"))
        title_label.pack(side="left")

        self.body_frame = ttk.Frame(self, padding=(15, 5, 5, 5))
        
        body_label = ttk.Label(self.body_frame, text=body, wraplength=550, justify="left")
        body_label.pack(fill="both", expand=True)

        self.header_frame.bind("<Button-1>", self._toggle)
        self.arrow_label.bind("<Button-1>", self._toggle)
        title_label.bind("<Button-1>", self._toggle)

    def _toggle(self, event=None):
        if self.body_frame.winfo_ismapped():
            self.arrow_label.config(text="▶ ")
            self.body_frame.pack_forget()
        else:
            self.arrow_label.config(text="▼ ")
            self.body_frame.pack(fill="x", expand=True)

class AppUI:
    def __init__(self, root_window, task_queue: Queue, ui_update_queue: Queue):
        self.root = root_window
        self.task_queue = task_queue
        self.ui_update_queue = ui_update_queue
        self.is_processing = False
        self.voice_controller = None
        self.core_worker = None
        self.browser_photo_image = None
        self.last_input_was_voice = False
        self.default_bg_color = None

        self.root.title("Universal Orchestrator (v6.1 - Final Fix)")
        self.root.geometry("1700x800")
        
        if sys.platform == "win32" and os.path.exists("logo.ico"):
            try: self.root.iconbitmap("logo.ico")
            except tk.TclError: print("[UI] [WARNING] Не удалось загрузить иконку 'logo.ico'.")
        
        style = ttk.Style(self.root)
        style.configure("Card.TFrame", background="#2E2E2E", borderwidth=1, relief="solid")
        style.configure("Card.TLabel", background="#2E2E2E", foreground="white")

        self.create_widgets()
        self.default_bg_color = self.chat_input.cget("background")
        
        self.root.after(100, self._process_ui_updates)
        self.root.after(3000, self._feedback_poll_loop)

    def _process_ui_updates(self):
        while not self.ui_update_queue.empty():
            try:
                message_type, args = self.ui_update_queue.get_nowait()
                if hasattr(self, message_type):
                    method = getattr(self, message_type)
                    method(*args)
                else:
                    self.log_to_widget(f"[UI] [ERROR] Получена неизвестная команда: {message_type}")
            except Empty:
                break
            except Exception as e:
                self.log_to_widget(f"[UI] [ERROR] Ошибка обработки UI-команды: {e}")
        self.root.after(100, self._process_ui_updates)

    def _open_settings_window(self):
        SettingsWindow(self.root, self)

    def set_core_worker(self, worker):
        self.core_worker = worker

    def request_core_reload(self):
        if self.core_worker:
            self.core_worker.trigger_reload()
        else:
            self.log_to_widget("[UI] Ошибка: CoreWorker не инициализирован.")

    def _insert_user_message(self, message, author_prefix):
        self.chat_area.config(state=tk.NORMAL)
        self.chat_area.insert(tk.END, f"{author_prefix}: {message}\n\n")
        self.chat_area.see(tk.END)
        self.chat_area.config(state=tk.DISABLED)

    def _insert_assistant_message(self, answer, thoughts):
        self.chat_area.config(state=tk.NORMAL)
        
        msg_frame = ttk.Frame(self.chat_area, padding=(0, 5, 0, 5))
        
        author_label = ttk.Label(msg_frame, text="Ассистент:", font=("TkDefaultFont", 9, "bold"))
        author_label.pack(anchor=tk.W, pady=(0, 2))

        if answer:
            answer_label = ttk.Label(msg_frame, text=answer, wraplength=600, justify=tk.LEFT)
            answer_label.pack(anchor=tk.W, fill=tk.X, expand=True, pady=(0, 5))

        if thoughts:
            collapsible = CollapsiblePane(msg_frame, text="Мысли", body=thoughts)
            collapsible.pack(fill=tk.X, expand=True, pady=(5, 5), padx=5)

        self.chat_area.window_create(tk.END, window=msg_frame)
        self.chat_area.insert(tk.END, '\n\n')
        
        self.chat_area.see(tk.END)
        self.chat_area.config(state=tk.DISABLED)

    def submit_task_from_input(self, event=None):
        if self.is_processing: return
        prompt = self.chat_input.get()
        if not prompt: return
        
        self.chat_input.delete(0, tk.END)
        author = "Вы (🎤)" if self.last_input_was_voice else "Вы"
        self._insert_user_message(prompt, author_prefix=author)
        self.task_queue.put(prompt)
        self.last_input_was_voice = False

    def create_widgets(self):
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
        if not self.root.winfo_exists(): return
        if is_listening:
            self.info_label.config(text="Слушаю вашу команду...")
            self.chat_input.config(background="#E0F7FF")
        else:
            self.info_label.config(text="Готов к работе. Ожидание задач...")
            self.chat_input.config(background=self.default_bg_color)

    def show_partial_transcription(self, text: str):
        self._update_input_text(text)

    def _update_input_text(self, text):
        self.chat_input.delete(0, tk.END)
        self.chat_input.insert(0, text)

    def set_ui_busy(self, is_busy, status_text=None):
        self.is_processing = is_busy
        if status_text: self.set_info_label(status_text)
        elif not is_busy: self.set_info_label("Готов к работе. Ожидание задач...")
        new_state = tk.DISABLED if is_busy else tk.NORMAL
        self.send_button.config(state=new_state)
        self.chat_input.config(state=new_state)
        if is_busy: self.progress_bar.start(10)
        else: self.progress_bar.stop()

    def unlock_settings_button(self):
        self.settings_button.config(state=tk.NORMAL)

    def set_info_label(self, text: str):
        self.info_label.config(text=text)

    def update_chat_with_final_result(self, final_result):
        if isinstance(final_result, str):
            structured_result = extract_thoughts(final_result)
            self._insert_assistant_message(structured_result['answer'], structured_result['thoughts'])
        elif isinstance(final_result, dict) and 'answer' in final_result:
            self._insert_assistant_message(final_result['answer'], final_result.get('thoughts'))
        else:
            self._insert_assistant_message("Получен неструктурированный ответ.", str(final_result))

    def log_to_widget(self, message):
        self._insert_log_message(message)

    def _insert_log_message(self, message):
        self.log_area.config(state=tk.NORMAL)
        self.log_area.insert(tk.END, str(message) + "\n")
        self.log_area.see(tk.END)
        self.log_area.config(state=tk.DISABLED)

    def _feedback_poll_loop(self):
        try:
            response = requests.get("http://127.0.0.1:7787/get_question", timeout=0.5)
            if response.status_code == 200 and (question := response.json().get("question")):
                self.log_to_widget(f"[Feedback] Получен вопрос для пользователя: {question}")
                self.root.after(0, self._ask_user_for_feedback, question)
        except requests.exceptions.RequestException: pass
        finally:
            if self.root.winfo_exists(): self.root.after(3000, self._feedback_poll_loop)

    def _ask_user_for_feedback(self, question: str):
        answer = simpledialog.askstring("Вопрос от Агента", question, parent=self.root)
        if answer is None: answer = "Пользователь отменил ввод."
        try:
            requests.post("http://127.0.0.1:7787/provide_answer", json={"answer": answer}, timeout=5)
            self.log_to_widget(f"[Feedback] Ответ '{answer}' отправлен агенту.")
        except requests.exceptions.RequestException as e: self.log_to_widget(f"[Feedback] [ERROR] Не удалось отправить ответ: {e}")
        
    def _pass_to_fortress(self, event): return handle_keypress_event(event, self.log_to_widget, self.chat_input)
        
    def _update_browser_view(self, b64_string: str):
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