import tkinter as tk
from tkinter import ttk, messagebox
import os
import json

SETTINGS_FILE = "settings.json"
STT_MODELS_PATH = "voice_engine/stt"
# ИЗМЕНЕНО: Добавляем словарь с моделями для выбора
TTS_ENGINES = {
    "silero": "Silero (Быстрый, базовый)",
    "xtts": "XTTS-v2 (Клонирование голоса)",
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
        selected_key = [k for k, v in TTS_ENGINES.items() if v == selected_engine_display][0]
        
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
                self.selected_tts_engine.set(TTS_ENGINES.get(engine_key, TTS_ENGINES["silero"]))
                
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
        engine_key = [k for k, v in TTS_ENGINES.items() if v == selected_engine_display][0]
        
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
            messagebox.showinfo("Успешно", "Настройки сохранены. Перезапустите голосовой движок (кнопка 🎤) для применения.", parent=self)
            self.destroy()
        except Exception as e: messagebox.showerror("Ошибка", f"Не удалось сохранить настройки: {e}", parent=self)
```<<<END_FILE>>>

<<<FILE: voice_engine/controller.py>>>
```py
# voice_engine/controller.py - ОБНОВЛЕН ДЛЯ РАБОТЫ С РАЗНЫМИ TTS ДВИЖКАМИ

import threading
import os
import json
from queue import Queue
import time
from .stt import SpeechToText
from .tts import TextToSpeech

SETTINGS_FILE = "settings.json"

class VoiceController:
    def __init__(self, orchestrator_engine):
        self.engine = orchestrator_engine
        self.ui_linker = None
        self.stt = None
        self.tts = None
        self.activation_word = ""
        
        self.tts_queue = Queue()
        self.tts_thread = None
        self.listener_thread = None
        self.is_running_event = threading.Event()
        self.tts_stop_event = threading.Event()
        
        self.reload()

    def _load_settings(self):
        if not os.path.exists(SETTINGS_FILE): return {}
        try:
            with open(SETTINGS_FILE, "r", encoding="utf-8") as f: return json.load(f)
        except Exception: return {}

    def reload(self):
        print("[VoiceController] Перезагрузка настроек голосового движка...")
        self.stop_listening()
        if self.tts_thread and self.tts_thread.is_alive():
            self.tts_queue.put(None); self.tts_thread.join(timeout=2)

        settings = self._load_settings()
        self.activation_word = settings.get("activation_word", "джарвис").lower().strip()

        try:
            # Инициализация STT
            stt_model = settings.get("stt_model")
            if stt_model: self.stt = SpeechToText(os.path.join("voice_engine", "stt", stt_model))

            # ИНИЦИАЛИЗАЦИЯ TTS (главное изменение)
            tts_engine = settings.get("tts_model_engine", "silero")
            tts_device = settings.get("tts_device", "cuda")
            self.tts = TextToSpeech(engine_id=tts_engine, device=tts_device, settings=settings)
            
            # Запускаем рабочие потоки
            self.tts_queue = Queue()
            self.tts_stop_event.clear()
            self.tts_thread = threading.Thread(target=self._tts_worker, daemon=True); self.tts_thread.start()
            
            if self.tts.is_ready: self.warm_up()
            self.start_listening()
            print("[VoiceController] Перезагрузка завершена.")
        except Exception as e:
            print(f"[VoiceController] [FATAL] Ошибка при перезагрузке компонентов: {e}", exc_info=True)

    def warm_up(self):
        if self.tts and self.tts.is_ready:
            print("[VoiceController] Выполняю прогрев TTS...")
            self.tts.speak("прогрев системы", self.tts_stop_event)
            print("[VoiceController] Прогрев TTS завершен.")
    
    def _tts_worker(self):
        while True:
            item = self.tts_queue.get()
            if item is None: break
            if self.tts_stop_event.is_set(): continue
            if self.tts and self.tts.is_ready:
                text_to_speak, _ = item
                self.tts.speak(text_to_speak, self.tts_stop_event)
            self.tts_queue.task_done()

    def _listen_loop(self):
        if not self.stt or not self.activation_word: return
        print(f"[VoiceController] Прослушивание активно. Слово активации: '{self.activation_word}'")
        is_activated, command_parts, utterance_buffer = False, [], ""

        for event_type, text in self.stt.listen():
            if not self.is_running_event.is_set(): break
            if not is_activated:
                if self.activation_word in text:
                    is_activated = True
                    self.tts_stop_event.set()
                    with self.tts_queue.mutex: self.tts_queue.queue.clear()
                    if self.ui_linker: self.ui_linker.set_listening_status(True)
                    utterance_buffer = text.split(self.activation_word, 1)[1].strip()
                    if self.ui_linker: self.ui_linker.show_partial_transcription(utterance_buffer)
            else:
                if event_type == 'partial':
                    utterance_buffer = text
                    if self.ui_linker: self.ui_linker.show_partial_transcription(" ".join(command_parts + [utterance_buffer]).strip())
                elif event_type == 'final':
                    if text: command_parts.append(text)
                    utterance_buffer = ""
                    if not text:
                        if self.ui_linker: self.ui_linker.set_listening_status(False)
                        final_command = " ".join(command_parts).strip()
                        if final_command and self.engine:
                            print(f"[VoiceController] Распознана команда: '{final_command}'")
                            if self.ui_linker: self.ui_linker.last_input_was_voice = True
                            self.engine.submit_task(final_command)
                        is_activated, command_parts = False, []
    
    def start_listening(self):
        if not self.is_running_event.is_set():
            self.is_running_event.set()
            if not self.listener_thread or not self.listener_thread.is_alive():
                self.listener_thread = threading.Thread(target=self._listen_loop, daemon=True)
                self.listener_thread.start()
    
    def stop_listening(self):
        if self.is_running_event.is_set(): self.is_running_event.clear()

    def say(self, text: str):
        if self.tts and self.tts.is_ready:
            self.tts_stop_event.clear()
            with self.tts_queue.mutex: self.tts_queue.queue.clear()
            self.tts_queue.put((text, time.time()))

    def set_ui_linker(self, ui_linker): self.ui_linker = ui_linker