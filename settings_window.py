# settings_window.py - Добавлен выбор устройства для TTS

import tkinter as tk
from tkinter import ttk, messagebox
import os
import json
import torch # Импортируем torch для проверки CUDA

SETTINGS_FILE = "settings.json"
STT_MODELS_PATH = "voice_engine/stt"
TTS_MODELS_PATH = "voice_engine/tts"
SILERO_SPEAKERS = ["aidar", "baya", "kseniya", "xenia", "eugene", "random"]

class SettingsWindow(tk.Toplevel):
    def __init__(self, parent, engine):
        super().__init__(parent)
        self.engine = engine
        self.transient(parent)
        self.title("Настройки голосового движка")
        self.geometry("500x400") # Увеличим высоту для нового элемента
        self.resizable(False, False)
        
        self.selected_stt = tk.StringVar()
        self.selected_tts = tk.StringVar()
        self.activation_word = tk.StringVar()
        self.assistant_name = tk.StringVar()
        self.tts_device = tk.StringVar() # Новая переменная для устройства

        self._create_widgets()
        self._load_models()
        self._load_settings()
        
        self.protocol("WM_DELETE_WINDOW", self._cancel)
        self.grab_set()

    def _create_widgets(self):
        main_frame = ttk.Frame(self, padding="10")
        main_frame.pack(fill=tk.BOTH, expand=True)
        
        activation_frame = ttk.LabelFrame(main_frame, text="Обращение и активация", padding="10")
        activation_frame.pack(fill=tk.X, pady=5)
        ttk.Label(activation_frame, text="Имя ассистента:").grid(row=0, column=0, sticky=tk.W, pady=2)
        ttk.Entry(activation_frame, textvariable=self.assistant_name).grid(row=0, column=1, sticky=tk.EW, pady=2)
        ttk.Label(activation_frame, text="Слово для активации:").grid(row=1, column=0, sticky=tk.W, pady=2)
        ttk.Entry(activation_frame, textvariable=self.activation_word).grid(row=1, column=1, sticky=tk.EW, pady=2)
        activation_frame.columnconfigure(1, weight=1)
        
        stt_frame = ttk.LabelFrame(main_frame, text="Распознавание речи (STT)", padding="10")
        stt_frame.pack(fill=tk.X, pady=5)
        ttk.Label(stt_frame, text="Модель Vosk:").pack(side=tk.LEFT, padx=(0, 10))
        self.stt_combobox = ttk.Combobox(stt_frame, textvariable=self.selected_stt, state="readonly")
        self.stt_combobox.pack(fill=tk.X, expand=True)
        
        tts_frame = ttk.LabelFrame(main_frame, text="Синтез речи (TTS)", padding="10")
        tts_frame.pack(fill=tk.X, pady=5)
        ttk.Label(tts_frame, text="Голос Silero:").pack(side=tk.LEFT, padx=(0, 10))
        self.tts_combobox = ttk.Combobox(tts_frame, textvariable=self.selected_tts, state="readonly")
        self.tts_combobox.pack(fill=tk.X, expand=True)

        # --- НОВЫЙ БЛОК: Выбор устройства ---
        device_frame = ttk.LabelFrame(main_frame, text="Устройство для синтеза", padding="10")
        device_frame.pack(fill=tk.X, pady=5)
        
        gpu_radio = ttk.Radiobutton(device_frame, text="GPU (NVIDIA, если доступно)", variable=self.tts_device, value="cuda")
        gpu_radio.pack(anchor=tk.W)
        
        cpu_radio = ttk.Radiobutton(device_frame, text="CPU (Центральный процессор)", variable=self.tts_device, value="cpu")
        cpu_radio.pack(anchor=tk.W)
        
        # Если CUDA недоступна, делаем выбор GPU неактивным
        if not torch.cuda.is_available():
            gpu_radio.config(state=tk.DISABLED)
            self.tts_device.set("cpu")
        # --- КОНЕЦ НОВОГО БЛОКА ---

        button_frame = ttk.Frame(main_frame)
        button_frame.pack(fill=tk.X, side=tk.BOTTOM, pady=(20, 0))
        ttk.Button(button_frame, text="Применить", command=self._save_and_apply).pack(side=tk.RIGHT)
        ttk.Button(button_frame, text="Отмена", command=self._cancel).pack(side=tk.RIGHT, padx=(0, 10))

    def _load_models(self):
        try:
            if os.path.isdir(STT_MODELS_PATH):
                stt_models = [d for d in os.listdir(STT_MODELS_PATH) if os.path.isdir(os.path.join(STT_MODELS_PATH, d))]
                self.stt_combobox['values'] = stt_models
            else: self.stt_combobox['values'] = ["Папка не найдена"]
        except Exception as e: self.stt_combobox['values'] = [f"Ошибка: {e}"]
        self.tts_combobox['values'] = SILERO_SPEAKERS

    def _load_settings(self):
        try:
            if os.path.exists(SETTINGS_FILE):
                with open(SETTINGS_FILE, "r", encoding="utf-8") as f: settings = json.load(f)
                self.selected_stt.set(settings.get("stt_model", ""))
                self.selected_tts.set(settings.get("tts_speaker", ""))
                self.activation_word.set(settings.get("activation_word", "ассистент"))
                self.assistant_name.set(settings.get("assistant_name", "Ассистент"))
                # Загружаем настройку устройства, по умолчанию 'cuda'
                self.tts_device.set(settings.get("tts_device", "cuda"))
            else:
                if self.stt_combobox['values']: self.selected_stt.set(self.stt_combobox['values'][0])
                if self.tts_combobox['values']: self.selected_tts.set(self.tts_combobox['values'][0])
                self.activation_word.set("ассистент")
                self.assistant_name.set("Ассистент")
                self.tts_device.set("cuda") # По умолчанию пытаемся использовать GPU
            
            # Дополнительная проверка, если CUDA недоступна
            if self.tts_device.get() == "cuda" and not torch.cuda.is_available():
                self.tts_device.set("cpu")

        except Exception as e: messagebox.showerror("Ошибка", f"Не удалось загрузить настройки: {e}", parent=self)

    def _save_and_apply(self):
        word = self.activation_word.get().strip().lower()
        name = self.assistant_name.get().strip()
        if not word or not name:
            messagebox.showwarning("Ошибка", "Поля имени и активации не могут быть пустыми.", parent=self)
            return

        settings = {
            "stt_model": self.selected_stt.get(),
            "tts_speaker": self.selected_tts.get(),
            "activation_word": word,
            "assistant_name": name,
            "tts_device": self.tts_device.get() # Сохраняем новое значение
        }
        try:
            with open(SETTINGS_FILE, "w", encoding="utf-8") as f:
                json.dump(settings, f, indent=2, ensure_ascii=False)
            
            if self.engine:
                self.engine.reload_settings()
            
            messagebox.showinfo("Успешно", "Настройки применены. Изменения вступят в силу после перезапуска голосового движка (выкл/вкл).", parent=self)
            self.destroy()
        except Exception as e:
            messagebox.showerror("Ошибка", f"Не удалось применить настройки: {e}", parent=self)

    def _cancel(self):
        self.destroy()