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