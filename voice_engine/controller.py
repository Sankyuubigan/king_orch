import threading
import os
import json
from queue import Queue
import time
import traceback
import logging
from .stt import SpeechToText
from .tts import TextToSpeech

SETTINGS_FILE = "settings.json"
logger = logging.getLogger(__name__)

class VoiceController:
    def __init__(self, task_queue: Queue):
        self.task_queue = task_queue
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
        logger.info("[VoiceController] Перезагрузка настроек голосового движка...")
        self.stop_listening()
        if self.tts_thread and self.tts_thread.is_alive():
            self.tts_queue.put(None)
            self.tts_thread.join(timeout=2)

        settings = self._load_settings()
        self.activation_word = settings.get("activation_word", "джарвис").lower().strip()

        try:
            stt_model_path = os.path.join("voice_engine", "stt", settings.get("stt_model", ""))
            if settings.get("stt_model") and os.path.isdir(stt_model_path):
                self.stt = SpeechToText(stt_model_path)
            else:
                self.stt = None
                logger.warning("[VoiceController] Модель STT не настроена или не найдена.")

            tts_engine_id = settings.get("tts_model_engine", "silero")
            tts_device = settings.get("tts_device", "cuda")
            self.tts = TextToSpeech(engine_id=tts_engine_id, device=tts_device, settings=settings)
            
            self.tts_queue = Queue()
            self.tts_stop_event.clear()
            self.tts_thread = threading.Thread(target=self._tts_worker, daemon=True)
            self.tts_thread.start()
            
            if self.tts.is_ready: self.warm_up()
            self.start_listening()
            logger.info("[VoiceController] Перезагрузка завершена.")
        except Exception as e:
            logger.critical(f"[VoiceController] КРИТИЧЕСКАЯ ОШИБКА при перезагрузке: {e}\n{traceback.format_exc()}")

    def warm_up(self):
        if self.tts and self.tts.is_ready:
            logger.info("[VoiceController] Выполняю прогрев TTS...")
            self.tts.speak("прогрев системы", self.tts_stop_event)
            logger.info("[VoiceController] Прогрев TTS завершен.")
    
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
        logger.info(f"[VoiceController] Прослушивание активно. Слово активации: '{self.activation_word}'")
        is_activated, command_parts, utterance_buffer = False, [], ""

        for event_type, text in self.stt.listen():
            if not self.is_running_event.is_set(): break
            if not is_activated:
                if self.activation_word in text:
                    is_activated = True
                    self.tts_stop_event.set()
                    with self.tts_queue.mutex: self.tts_queue.queue.clear()
                    if self.ui_linker: self.ui_linker.set_listening_status(True)
                    utterance_buffer = text.split(self.activation_word, 1).strip()
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
                        if final_command:
                            logger.info(f"[VoiceController] Распознана команда: '{final_command}'")
                            if self.ui_linker: self.ui_linker.last_input_was_voice = True
                            self.task_queue.put(final_command) # <-- ИЗМЕНЕНО: Отправка в очередь
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