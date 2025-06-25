import tkinter as tk
from tkinter import ttk, messagebox
import os
import json

SETTINGS_FILE = "settings.json"
STT_MODELS_PATH = "voice_engine/stt"
# --- ИЗМЕНЕНИЕ: Удален движок XTTS из словаря ---
TTS_ENGINES = {
    "silero": "Silero (Быстрый, базовый)",
    "f5": "F5-TTS (Качественный русский)"
}
SILERO_SPEAKERS = ["aidar", "baya", "kseniya", "xenia", "eugene", "random"]

class SettingsWindow(tk.Toplevel):
    def __init__(self, parent, engine):
        super().__init__(parent)
        self.engine = engine
        self.title("Настройки голосового движка")
        self.geometry("500x450")
        
        self.selected_stt = tk.StringVar()
        self.selected_tts_engine = tk.StringVar() # Для выбора движка
        self.selected_silero_speaker = tk.StringVar() # Для выбора голоса Silero
        self.activation_word = tk.StringVar()
        self.assistant_name = tk.StringVar()
        self.tts_device = tk.StringVar()

        self._create_widgets()
        self._load_models()
        self._load_settings()
        
        self.grab_set()

    def _create_widgets(self):
        main_frame = ttk.Frame(self, padding="10")
        main_frame.pack(fill=tk.BOTH, expand=True)
        
        # ... (код активации без изменений) ...
        activation_frame = ttk.LabelFrame(main_frame, text="Обращение и активация", padding="10")
        activation_frame.pack(fill=tk.X, pady=5)
        ttk.Label(activation_frame, text="Имя ассистента:").grid(row=0, column=0, sticky=tk.W, pady=2)
        ttk.Entry(activation_frame, textvariable=self.assistant_name).grid(row=0, column=1, sticky=tk.EW, pady=2)
        ttk.Label(activation_frame, text="Слово для активации:").grid(row=1, column=0, sticky=tk.W, pady=2)
        ttk.Entry(activation_frame, textvariable=self.activation_word).grid(row=1, column=1, sticky=tk.EW, pady=2)
        activation_frame.columnconfigure(1, weight=1)

        stt_frame = ttk.LabelFrame(main_frame, text="Распознавание речи (STT)", padding="10")
        stt_frame.pack(fill=tk.X, pady=5)
        self.stt_combobox = ttk.Combobox(stt_frame, textvariable=self.selected_stt, state="readonly")
        self.stt_combobox.pack(fill=tk.X, expand=True)
        
        tts_frame = ttk.LabelFrame(main_frame, text="Синтез речи (TTS)", padding="10")
        tts_frame.pack(fill=tk.X, pady=5)

        ttk.Label(tts_frame, text="Движок:").grid(row=0, column=0, sticky=tk.W, pady=(0,5))
        self.tts_engine_combobox = ttk.Combobox(tts_frame, textvariable=self.selected_tts_engine, state="readonly", values=list(TTS_ENGINES.values()))
        self.tts_engine_combobox.grid(row=0, column=1, sticky=tk.EW, pady=(0,5))

        self.silero_speaker_label = ttk.Label(tts_frame, text="Голос (Silero):")
        self.silero_speaker_label.grid(row=1, column=0, sticky=tk.W)
        self.silero_speaker_combobox = ttk.Combobox(tts_frame, textvariable=self.selected_silero_speaker, state="readonly", values=SILERO_SPEAKERS)
        self.silero_speaker_combobox.grid(row=1, column=1, sticky=tk.EW)
        
        ttk.Label(tts_frame, text="Устройство:").grid(row=2, column=0, sticky=tk.W, pady=(5,0))
        device_subframe = ttk.Frame(tts_frame)
        device_subframe.grid(row=2, column=1, sticky=tk.EW, pady=(5,0))
        self.gpu_radio = ttk.Radiobutton(device_subframe, text="GPU", variable=self.tts_device, value="cuda")
        self.gpu_radio.pack(side=tk.LEFT, expand=True)
        self.cpu_radio = ttk.Radiobutton(device_subframe, text="CPU", variable=self.tts_device, value="cpu")
        self.cpu_radio.pack(side=tk.LEFT, expand=True)

        tts_frame.columnconfigure(1, weight=1)

        button_frame = ttk.Frame(main_frame)
        button_frame.pack(fill=tk.X, side=tk.BOTTOM, pady=(20, 0))
        ttk.Button(button_frame, text="Применить", command=self._save_and_apply).pack(side=tk.RIGHT)
        ttk.Button(button_frame, text="Отмена", command=self.destroy).pack(side=tk.RIGHT, padx=(0, 10))

        self.tts_engine_combobox.bind("<<ComboboxSelected>>", self._on_engine_change)

    def _on_engine_change(self, event=None):
        """Скрывает/показывает выбор голоса Silero."""
        selected_engine_display = self.selected_tts_engine.get()
        # Находим ключ по значению
        selected_key = ""
        for k, v in TTS_ENGINES.items():
            if v == selected_engine_display:
                selected_key = k
                break
        
        if selected_key == "silero":
            self.silero_speaker_label.grid()
            self.silero_speaker_combobox.grid()
        else:
            self.silero_speaker_label.grid_remove()
            self.silero_speaker_combobox.grid_remove()

    def _load_models(self):
        try:
            stt_models = [d for d in os.listdir(STT_MODELS_PATH) if os.path.isdir(os.path.join(STT_MODELS_PATH, d))] if os.path.isdir(STT_MODELS_PATH) else ["Папка не найдена"]
            self.stt_combobox['values'] = stt_models
        except Exception as e: self.stt_combobox['values'] = [f"Ошибка: {e}"]

    def _load_settings(self):
        try:
            if os.path.exists(SETTINGS_FILE):
                with open(SETTINGS_FILE, "r", encoding="utf-8") as f: settings = json.load(f)
                self.selected_stt.set(settings.get("stt_model", ""))
                # Загрузка движка TTS
                engine_key = settings.get("tts_model_engine", "silero")
                # Убедимся, что сохраненный движок все еще доступен
                if engine_key not in TTS_ENGINES:
                    engine_key = "silero" # Откат на дефолтный, если сохраненный был удален
                self.selected_tts_engine.set(TTS_ENGINES.get(engine_key))
                
                self.selected_silero_speaker.set(settings.get("tts_silero_speaker", "aidar"))
                self.activation_word.set(settings.get("activation_word", "джарвис"))
                self.assistant_name.set(settings.get("assistant_name", "Ассистент"))
                self.tts_device.set(settings.get("tts_device", "cuda"))
            else: # Дефолтные значения
                if self.stt_combobox['values']: self.selected_stt.set(self.stt_combobox['values'][0])
                self.selected_tts_engine.set(TTS_ENGINES["silero"])
                self.selected_silero_speaker.set("aidar")
                self.activation_word.set("джарвис")
                self.assistant_name.set("Ассистент")
                self.tts_device.set("cuda")
        except Exception as e: messagebox.showerror("Ошибка", f"Не удалось загрузить настройки: {e}", parent=self)
        self._on_engine_change() # Обновляем UI

    def _save_and_apply(self):
        selected_engine_display = self.selected_tts_engine.get()
        engine_key = ""
        for k, v in TTS_ENGINES.items():
            if v == selected_engine_display:
                engine_key = k
                break
        
        settings = {
            "stt_model": self.selected_stt.get(),
            "tts_model_engine": engine_key,
            "tts_silero_speaker": self.selected_silero_speaker.get(),
            "activation_word": self.activation_word.get().strip().lower(),
            "assistant_name": self.assistant_name.get().strip(),
            "tts_device": self.tts_device.get()
        }
        try:
            with open(SETTINGS_FILE, "w", encoding="utf-8") as f:
                json.dump(settings, f, indent=2, ensure_ascii=False)
            
            if self.engine: self.engine.reload_settings()
            messagebox.showinfo("Успешно", "Настройки сохранены. Изменения вступят в силу немедленно или после перезапуска голосового движка.", parent=self)
            self.destroy()
        except Exception as e: messagebox.showerror("Ошибка", f"Не удалось сохранить настройки: {e}", parent=self)