# ui.py - Добавлен переключатель для голосового движка

import threading
import tkinter as tk
from tkinter import ttk, scrolledtext, messagebox, simpledialog
import time
import requests
from io import BytesIO
from PIL import Image, ImageTk

from engine import OrchestratorEngine
from utils.clipboard_fortress import handle_keypress_event

FEEDBACK_URL_GET = "http://127.0.0.1:7787/get_question"
FEEDBACK_URL_POST = "http://127.0.0.1:7787/provide_answer"
STATUS_ICONS = {"pending": "⏳", "running": "⚙️", "done": "✅", "failed": "❌", "fixing": "🛠️"}

class AppUI:
    def __init__(self, root_window, engine: OrchestratorEngine):
        self.root = root_window
        self.engine = engine
        self.is_processing = False
        self.plan_widgets = {}
        # --- НОВОЕ: Переменная для состояния переключателя ---
        self.voice_engine_enabled = tk.BooleanVar(value=False)

        self.root.title("The Orchestrator v30.0 (Voice Control)")
        self.root.geometry("1700x800")
        
        self.create_widgets()
        
        self.engine.log_callback = self.log_to_widget
        self.engine.update_callback = self._handle_engine_update
        self.engine.busy_callback = self.set_ui_busy
        
        self.start_initial_load_task()
        self.root.after(1000, self._feedback_poll_loop)

    # --- НОВЫЙ МЕТОД: Обработчик переключателя ---
    def _toggle_voice_engine(self):
        is_enabled = self.voice_engine_enabled.get()
        self.engine.toggle_voice_engine(is_enabled)
        status = "включен" if is_enabled else "выключен"
        self.log_to_widget(f"[UI] Голосовой ввод {status}.")

    def _handle_engine_update(self, message: dict):
        if self.root.winfo_exists():
            self.root.after(0, self._process_update_in_main_thread, message)

    def _process_update_in_main_thread(self, message: dict):
        # ... (код без изменений)
        msg_type = message.get("type")
        data = message.get("data")

        if msg_type == "user_prompt":
            self._insert_chat_message(data, is_user=True)
        elif msg_type == "plan":
            self._render_plan(data)
        elif msg_type == "status_update":
            self._update_plan_item(data)
        elif msg_type == "final_result":
            self._finalize_chat_response(data)
        elif msg_type == "error":
            self._insert_chat_message(f"Ошибка: {data}", is_user=False)

    def _render_plan(self, plan_data: list):
        # ... (код без изменений)
        self.plan_widgets.clear()
        self.chat_area.config(state=tk.NORMAL)
        plan_container = ttk.Frame(self.chat_area, padding=5, relief="solid", borderwidth=1)
        title_label = ttk.Label(plan_container, text="План выполнения задачи:", font=("Segoe UI", 10, "bold"))
        title_label.pack(fill=tk.X, padx=5, pady=(5, 10))
        for step in plan_data:
            step_id = step["id"]
            step_frame = ttk.Frame(plan_container)
            step_frame.pack(fill=tk.X, padx=5, pady=2)
            icon_label = ttk.Label(step_frame, text=STATUS_ICONS.get(step["status"], "❓"), font=("Segoe UI", 10))
            icon_label.pack(side=tk.LEFT, padx=(0, 5))
            desc_label = ttk.Label(step_frame, text=step["description"], wraplength=700, anchor="w", justify=tk.LEFT)
            desc_label.pack(side=tk.LEFT, fill=tk.X, expand=True)
            self.plan_widgets[step_id] = {"icon": icon_label, "desc": desc_label}
        self.chat_area.window_create(tk.END, window=plan_container)
        self.chat_area.insert(tk.END, "\n\n")
        self.chat_area.config(state=tk.DISABLED)
        self.chat_area.see(tk.END)

    def _update_plan_item(self, update_data: dict):
        # ... (код без изменений)
        step_id = update_data["id"]
        new_status = update_data["status"]
        if step_id in self.plan_widgets:
            widgets = self.plan_widgets[step_id]
            widgets["icon"].config(text=STATUS_ICONS.get(new_status, "❓"))
            style_map = {"failed": "red", "done": "green", "fixing": "orange"}
            color = style_map.get(new_status)
            widgets["desc"].config(foreground=color if color else "")

    def _feedback_poll_loop(self):
        # ... (код без изменений)
        try:
            response = requests.get(FEEDBACK_URL_GET, timeout=0.5)
            if response.status_code == 200:
                data = response.json()
                if question := data.get("question"):
                    self.log_to_widget(f"[Feedback] Получен вопрос для пользователя: {question}")
                    self.root.after(0, self._ask_user_for_feedback, question)
        except requests.exceptions.RequestException:
            pass
        finally:
            self.root.after(3000, self._feedback_poll_loop)

    def _ask_user_for_feedback(self, question: str):
        # ... (код без изменений)
        answer = simpledialog.askstring("Вопрос от Агента", question, parent=self.root)
        if answer is None: answer = "Пользователь отменил ввод."
        try:
            requests.post(FEEDBACK_URL_POST, json={"answer": answer}, timeout=5)
            self.log_to_widget(f"[Feedback] Ответ '{answer}' отправлен агенту.")
        except requests.exceptions.RequestException as e:
            self.log_to_widget(f"[Feedback] [ERROR] Не удалось отправить ответ: {e}")

    def _pass_to_fortress(self, event):
        # ... (код без изменений)
        return handle_keypress_event(event, self.log_to_widget, self.chat_input)

    def _copy_logs_to_clipboard(self):
        # ... (код без изменений)
        try:
            logs = self.log_area.get("1.0", tk.END)
            self.root.clipboard_clear(); self.root.clipboard_append(logs)
            self.log_to_widget("[UI] Все логи скопированы в буфер обмена.")
        except Exception as e:
            self.log_to_widget(f"[UI] [ERROR] Не удалось скопировать логи: {e}")

    def create_widgets(self):
        self.main_pane = ttk.PanedWindow(self.root, orient=tk.HORIZONTAL)
        self.main_pane.pack(fill=tk.BOTH, expand=True)
        left_pane_container = ttk.Frame(self.main_pane, padding=5)
        self.main_pane.add(left_pane_container, weight=1)
        top_frame = ttk.Frame(left_pane_container)
        top_frame.pack(side=tk.TOP, fill=tk.X, pady=5)
        self.info_label = ttk.Label(top_frame, text="Загрузка...")
        self.info_label.pack(side=tk.LEFT, pady=5, padx=5, fill=tk.X, expand=True)
        
        # --- НОВЫЙ ВИДЖЕТ: Переключатель голосового ввода ---
        voice_toggle_button = ttk.Checkbutton(
            top_frame,
            text="Голосовой ввод 🎤",
            variable=self.voice_engine_enabled,
            command=self._toggle_voice_engine,
            style="Toolbutton" # Стиль, чтобы выглядело как кнопка
        )
        voice_toggle_button.pack(side=tk.RIGHT, padx=5)

        notebook = ttk.Notebook(left_pane_container)
        # ... (остальной код создания виджетов без изменений)
        notebook.pack(fill=tk.BOTH, expand=True, side=tk.TOP)
        chat_tab = ttk.Frame(notebook); log_tab = ttk.Frame(notebook)
        notebook.add(chat_tab, text='Чат'); notebook.add(log_tab, text='Логи')
        log_frame = ttk.Frame(log_tab)
        log_frame.pack(fill=tk.BOTH, expand=True)
        self.log_area = scrolledtext.ScrolledText(log_frame, wrap=tk.WORD, state=tk.DISABLED)
        self.log_area.pack(side=tk.TOP, fill=tk.BOTH, expand=True)
        copy_logs_button = ttk.Button(log_frame, text="Копировать все логи", command=self._copy_logs_to_clipboard)
        copy_logs_button.pack(side=tk.BOTTOM, fill=tk.X, pady=(5,0))
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
        self.chat_input.focus_set()

    def _insert_message_with_trajectory(self, prefix, message, trajectory):
        # ... (код без изменений)
        self.chat_area.config(state=tk.NORMAL)
        self.chat_area.insert(tk.END, f"{prefix}: {message}\n")
        if trajectory:
            pass
        self.chat_area.insert(tk.END, "\n\n")
        self.chat_area.config(state=tk.DISABLED)
        self.chat_area.see(tk.END)

    def start_initial_load_task(self):
        # ... (код без изменений)
        self.set_ui_busy(True, "Первоначальная загрузка модели...")
        threading.Thread(target=self.engine.initial_load, daemon=True).start()

    def set_ui_busy(self, is_busy, status_text=None):
        # ... (код без изменений)
        self.is_processing = is_busy
        if status_text: self.info_label.config(text=status_text)
        self.send_button.config(state=tk.DISABLED if is_busy else tk.NORMAL)
        self.chat_input.config(state=tk.DISABLED if is_busy else tk.NORMAL)
        if is_busy: self.progress_bar.start()
        else: self.progress_bar.stop()

    def _finalize_chat_response(self, response_data):
        # ... (код без изменений)
        final_answer = response_data.get("final_result", "[Агент не вернул финальный ответ]")
        trajectory = response_data.get("trajectory", [])
        self._insert_message_with_trajectory("Модель (🔊):", final_answer, trajectory)
    
    def _insert_chat_message(self, message, is_user=True):
        # ... (код без изменений)
        self.chat_area.config(state=tk.NORMAL)
        prefix = "Вы (🎤):" if is_user else "Модель (🔊):"
        self.chat_area.insert(tk.END, f"{prefix} {message}\n\n")
        self.chat_area.see(tk.END)
        self.chat_area.config(state=tk.DISABLED)

    def start_chat_task(self, event=None):
        # ... (код без изменений)
        prompt = self.chat_input.get()
        if not prompt or self.is_processing: return
        self.chat_input.delete(0, tk.END)
        self.engine.submit_task(prompt)
        
    def log_to_widget(self, message):
        # ... (код без изменений)
        if self.root.winfo_exists(): self.root.after(0, self._insert_log_message, message)

    def _insert_log_message(self, message):
        # ... (код без изменений)
        self.log_area.config(state=tk.NORMAL)
        self.log_area.insert(tk.END, str(message) + "\n")
        self.log_area.see(tk.END)
        self.log_area.config(state=tk.DISABLED)