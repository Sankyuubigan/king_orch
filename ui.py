# ui.py - СКРЫТИЕ ДЕТАЛЕЙ ДЛЯ ПРЯМЫХ ОТВЕТОВ

import threading
import tkinter as tk
from tkinter import ttk, scrolledtext, messagebox
import time
import requests
from io import BytesIO
from PIL import Image, ImageTk

from engine import OrchestratorEngine
from utils.clipboard_fortress import handle_keypress_event

SCREENSHOT_URL = "http://127.0.0.1:7777/screenshot"

class AppUI:
    def __init__(self, root_window, engine: OrchestratorEngine):
        self.root = root_window
        self.engine = engine
        self.browser_visible = False
        self.is_processing = False
        self.screenshot_thread = None
        self.stop_screenshot_thread = threading.Event()

        self.root.title("The Orchestrator v19.1 (Stable)")
        self.root.geometry("1700x800")
        
        self.create_widgets()
        self.root.after(100, self.toggle_browser_visibility)
        self.start_load_model_task()

    def _pass_to_fortress(self, event):
        """Передает событие в изолированный обработчик."""
        return handle_keypress_event(event, self.log_to_widget, self.chat_input)

    def create_widgets(self):
        self.main_pane = ttk.PanedWindow(self.root, orient=tk.HORIZONTAL)
        self.main_pane.pack(fill=tk.BOTH, expand=True)
        left_pane_container = ttk.Frame(self.main_pane, padding=5)
        self.main_pane.add(left_pane_container, weight=1)
        top_frame = ttk.Frame(left_pane_container)
        top_frame.pack(side=tk.TOP, fill=tk.X, pady=5)
        self.info_label = ttk.Label(top_frame, text="Модель загружается...")
        self.info_label.pack(side=tk.LEFT, pady=5, padx=5, fill=tk.X, expand=True)
        self.browser_toggle_button = ttk.Button(top_frame, text="Скрыть браузер", command=self.toggle_browser_visibility)
        self.browser_toggle_button.pack(side=tk.RIGHT, pady=5, padx=5)
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
        self.send_button = ttk.Button(input_frame, text="Отправить", command=self.start_chat_task)
        self.send_button.pack(side=tk.RIGHT)

        self.chat_input.bind("<Return>", self.start_chat_task)
        for widget in [self.chat_input, self.log_area, self.chat_area]:
            widget.bind("<KeyPress>", self._pass_to_fortress)

        status_bar = ttk.Frame(left_pane_container, padding=(5, 5))
        status_bar.pack(side=tk.BOTTOM, fill=tk.X)
        self.progress_bar = ttk.Progressbar(status_bar, mode='indeterminate', length=150)
        self.progress_bar.pack(side=tk.RIGHT)
        self.browser_container = ttk.Frame(self.main_pane)
        self.screenshot_label = ttk.Label(self.browser_container, text="Ожидание скриншота...", anchor="center")
        self.screenshot_label.pack(fill=tk.BOTH, expand=True)
        
    def _insert_message_with_trajectory(self, prefix, message, trajectory):
        self.chat_area.config(state=tk.NORMAL)
        self.chat_area.insert(tk.END, f"{prefix}: {message}\n")

        # ИСПРАВЛЕНИЕ: Не показываем детали, если ответ сгенерирован напрямую
        direct_answer_traj = ["Ответ сгенерирован напрямую, без использования команд."]
        if trajectory and trajectory != direct_answer_traj:
            traj_frame = ttk.Frame(self.chat_area, padding=2)
            traj_text_widget = scrolledtext.ScrolledText(traj_frame, wrap=tk.WORD, height=8, width=100, font=("Consolas", 9))
            traj_text_widget.insert(tk.END, "\n".join(trajectory))
            traj_text_widget.config(state=tk.DISABLED)
            traj_text_widget.bind("<KeyPress>", self._pass_to_fortress)
            def toggle_traj():
                if traj_text_widget.winfo_viewable():
                    traj_text_widget.pack_forget()
                    toggle_button.config(text="Показать детали 🔽")
                else:
                    traj_text_widget.pack(fill=tk.BOTH, expand=True, pady=(2,0))
                    toggle_button.config(text="Скрыть детали 🔼")
            toggle_button = ttk.Button(traj_frame, text="Показать детали 🔽", command=toggle_traj, style="Toolbutton")
            toggle_button.pack(fill=tk.X)
            self.chat_area.window_create(tk.END, window=traj_frame)

        self.chat_area.insert(tk.END, "\n\n")
        self.chat_area.config(state=tk.DISABLED)
        self.chat_area.see(tk.END)

    def start_load_model_task(self):
        self.set_ui_busy(True, "Загрузка модели...")
        threading.Thread(target=self._load_model_thread, daemon=True).start()

    def _load_model_thread(self):
        success = self.engine.load_model()
        if self.root.winfo_exists():
            if success:
                status_text = f"Готов к работе. Модель: {self.engine.model_name}"
                self.root.after(0, self.set_ui_busy, False, status_text)
            else:
                self.root.after(0, self.set_ui_busy, False, "ОШИБКА ЗАГРУЗКИ МОДЕЛИ!")
                self.root.after(0, lambda: messagebox.showerror("Ошибка", "Не удалось загрузить модель. Проверьте логи."))

    def set_ui_busy(self, is_busy, status_text=None):
        self.is_processing = is_busy
        state = tk.DISABLED if is_busy else tk.NORMAL
        if status_text: self.info_label.config(text=status_text)
        if self.send_button.winfo_exists():
            self.send_button.config(state=state)
            self.chat_input.config(state=state)
            if is_busy: self.progress_bar.start()
            else: self.progress_bar.stop()

    def _finalize_chat_response(self, response_data):
        final_answer = response_data.get("final_result", "[Агент не вернул финальный ответ]")
        trajectory = response_data.get("trajectory", [])
        self._insert_message_with_trajectory("Модель", final_answer, trajectory)
        status_text = f"Готов к работе. Модель: {self.engine.model_name}"
        self.set_ui_busy(False, status_text)
    
    def toggle_browser_visibility(self):
        if self.browser_visible:
            self.stop_screenshot_thread.set()
            if self.browser_container.winfo_ismapped(): self.main_pane.forget(self.browser_container)
            self.browser_toggle_button.config(text="Показать браузер")
        else:
            self.main_pane.add(self.browser_container, weight=2)
            self.browser_toggle_button.config(text="Скрыть браузер")
            self.stop_screenshot_thread.clear()
            self.screenshot_thread = threading.Thread(target=self._screenshot_loop, daemon=True)
            self.screenshot_thread.start()
            self.root.after(100, lambda: self.main_pane.sashpos(0, 600))
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
                self.root.after(0, self.screenshot_label.config, {"image": photo, "text": ""})
                self.screenshot_label.image = photo
            except Exception:
                self.root.after(0, self.screenshot_label.config, {"image": None, "text": "Не удалось получить скриншот..."})
                time.sleep(1)
            time.sleep(3)

    def _insert_chat_message(self, message):
        self.chat_area.config(state=tk.NORMAL)
        self.chat_area.insert(tk.END, message + "\n\n")
        self.chat_area.see(tk.END)
        self.chat_area.config(state=tk.DISABLED)

    def start_chat_task(self, event=None):
        prompt = self.chat_input.get()
        if not prompt or self.is_processing: return
        if not self.engine.llm:
            messagebox.showwarning("Внимание", "Модель еще не загружена или произошла ошибка загрузки. Пожалуйста, подождите.")
            return
        self.chat_input.delete(0, tk.END)
        self._insert_chat_message(f"Вы: {prompt}")
        self.set_ui_busy(True, "Команда агентов работает...")
        threading.Thread(target=self._get_engine_decision, args=(prompt,), daemon=True).start()

    def _get_engine_decision(self, prompt):
        response = self.engine.get_response(prompt)
        if self.root.winfo_exists(): self.root.after(0, self._process_engine_decision, response)

    def _process_engine_decision(self, response):
        status = response.get("status")
        if status == "done":
            self._finalize_chat_response(response["content"])
        else:
            self._insert_chat_message(f"Ошибка: {response['content']}")
            status_text = f"Готов к работе. Модель: {self.engine.model_name}"
            self.set_ui_busy(False, status_text)
        
    def log_to_widget(self, message):
        if self.root.winfo_exists(): self.root.after(0, self._insert_log_message, message)

    def _insert_log_message(self, message):
        self.log_area.config(state=tk.NORMAL)
        self.log_area.insert(tk.END, message + "\n")
        self.log_area.see(tk.END)
        self.log_area.config(state=tk.DISABLED)