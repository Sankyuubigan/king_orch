# voice_engine/controller.py - Исправление пути к модели Vosk

import threading
from .stt import SpeechToText
from .tts import TextToSpeech

class VoiceController:
    """
    Управляет процессами распознавания (STT) и синтеза (TTS) речи,
    реализует логику прерывания (Barge-in) и имеет внешний контроль.
    """
    def __init__(self, orchestrator_engine):
        print("[VoiceController] Инициализация голосового движка...")
        self.engine = orchestrator_engine
        
        # --- ИСПРАВЛЕНО: Путь должен указывать прямо на папку с файлами модели ---
        vosk_model_path = "voice_engine/vosk/vosk-model-tts-ru-0.8-multi"
        silero_model_path = "voice_engine/silero"
        
        self.stt = SpeechToText(model_path=vosk_model_path)
        self.tts = TextToSpeech(model_base_path=silero_model_path)
        
        self.tts_stop_event = threading.Event()
        self.tts_thread = None
        self.tts_lock = threading.Lock()
        
        self.is_running_event = threading.Event()
        self.listener_thread = None
        
        print("[VoiceController] Голосовой движок инициализирован и готов к запуску.")

    def _listen_loop(self):
        """
        Бесконечный цикл, который слушает команды пользователя.
        Работает, пока установлен is_running_event.
        """
        print("[VoiceController] Цикл прослушивания запущен.")
        while self.is_running_event.is_set():
            recognized_text = self.stt.listen()
            
            if not self.is_running_event.is_set(): break
            if recognized_text:
                print(f"[VoiceController] Распознана команда: '{recognized_text}'")
                if self.tts_thread and self.tts_thread.is_alive():
                    print("[VoiceController] Barge-in! Прерываю текущую речь.")
                    self.tts_stop_event.set()
                    self.tts_thread.join()
                self.engine.submit_task(recognized_text)
        print("[VoiceController] Цикл прослушивания остановлен.")

    def start_listening(self):
        """Запускает прослушивание, если оно еще не запущено."""
        if not self.is_running_event.is_set():
            self.is_running_event.set()
            if self.listener_thread is None or not self.listener_thread.is_alive():
                self.listener_thread = threading.Thread(target=self._listen_loop)
                self.listener_thread.daemon = True
                self.listener_thread.start()

    def stop_listening(self):
        """Останавливает прослушивание."""
        if self.is_running_event.is_set():
            print("[VoiceController] Получена команда на остановку прослушивания.")
            self.is_running_event.clear()

    def say(self, text: str):
        """Публичный метод для озвучивания текста."""
        with self.tts_lock:
            if self.tts_thread and self.tts_thread.is_alive():
                self.tts_thread.join()
            self.tts_stop_event.clear()
            self.tts_thread = threading.Thread(target=self.tts.speak, args=(text, self.tts_stop_event))
            self.tts_thread.start()