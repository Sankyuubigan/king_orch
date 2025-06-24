# voice_engine/controller.py - ИСПРАВЛЕНА ЛОГИКА ГОЛОСОВОЙ АКТИВАЦИИ

import threading
import os
import json
import re
import time
from queue import Queue
from .stt import SpeechToText
from .tts import TextToSpeech

SETTINGS_FILE = "settings.json"
WARM_UP_COMMAND = "Это тестовое предложение для прогрева системы синтеза речи."

class VoiceController:
    def __init__(self, orchestrator_engine):
        self.engine = orchestrator_engine
        self.ui_linker = None
        self.stt = None
        self.tts = None
        self.activation_word = ""
        
        self.tts_stop_event = threading.Event()
        self.tts_queue = Queue()
        self.tts_thread = None
        
        self.is_running_event = threading.Event()
        self.listener_thread = None
        
        self.reload()

    def set_ui_linker(self, ui_linker):
        self.ui_linker = ui_linker

    def reload(self):
        print("[VoiceController] Перезагрузка настроек...")
        self.stop_listening()
        if self.tts_thread and self.tts_thread.is_alive():
            self.tts_queue.put(None)
            self.tts_thread.join(timeout=2)

        settings = self._load_settings()
        stt_model_name = settings.get("stt_model")
        tts_speaker_name = settings.get("tts_speaker")
        self.activation_word = settings.get("activation_word", "").lower().strip()
        tts_device = settings.get("tts_device", "cuda")

        try:
            if stt_model_name:
                stt_model_path = os.path.join("voice_engine", "stt", stt_model_name)
                self.stt = SpeechToText(model_path=stt_model_path)
            if tts_speaker_name:
                tts_model_base_path = os.path.join("voice_engine", "tts")
                self.tts = TextToSpeech(model_base_path=tts_model_base_path, speaker=tts_speaker_name, device=tts_device)
        except Exception as e:
            print(f"[VoiceController] [ERROR] Ошибка при пересоздании компонентов: {e}")
            return

        self.tts_queue = Queue()
        self.tts_stop_event.clear()
        self.tts_thread = threading.Thread(target=self._tts_worker, daemon=True)
        self.tts_thread.start()
        
        self.warm_up()
        
        self.start_listening()
        print("[VoiceController] Перезагрузка завершена.")

    def warm_up(self):
        if self.tts:
            print("[VoiceController] Выполняю блокирующий прогрев TTS...")
            start_time = time.time()
            self.tts.speak(WARM_UP_COMMAND, threading.Event(), is_warm_up=True)
            end_time = time.time()
            print(f"[VoiceController] Прогрев TTS завершен за {end_time - start_time:.2f} сек.")

    def _load_settings(self) -> dict:
        if os.path.exists(SETTINGS_FILE):
            try:
                with open(SETTINGS_FILE, "r", encoding="utf-8") as f: return json.load(f)
            except Exception: return {}
        return {}

    def _tts_worker(self):
        while True:
            item = self.tts_queue.get()
            if item is None: break
            
            text_to_speak, _ = item
            if self.tts_stop_event.is_set(): continue
            
            if self.tts:
                self.tts.speak(text_to_speak, self.tts_stop_event, is_warm_up=False)
            
            self.tts_queue.task_done()

    def _listen_loop(self):
        """
        Полностью переписанная, надежная логика цикла прослушивания.
        """
        if not self.stt or not self.activation_word:
            print("[VoiceController] Прослушивание неактивно: STT или слово активации не заданы.")
            return
            
        print(f"[VoiceController] Цикл прослушивания запущен. Слово активации: '{self.activation_word}'")
        
        is_activated = False
        command_parts = []
        # Буфер для текста из текущей распознаваемой фразы (до паузы)
        current_utterance_buffer = ""

        for event_type, text in self.stt.listen():
            if not self.is_running_event.is_set(): break

            if not is_activated:
                # --- СОСТОЯНИЕ: ОЖИДАНИЕ АКТИВАЦИИ ---
                if self.activation_word in text:
                    print(f"[VoiceController] Обнаружено активационное слово!")
                    is_activated = True
                    
                    # Barge-In: прерываем текущую речь ассистента
                    if not self.tts_queue.empty() or (self.tts_thread and self.tts_thread.is_alive()):
                        print("[VoiceController] Barge-In! Очистка очереди TTS.")
                        self.tts_stop_event.set()
                        with self.tts_queue.mutex: self.tts_queue.queue.clear()
                    
                    if self.ui_linker: self.ui_linker.set_listening_status(True)

                    # Извлекаем текст, который был произнесен вместе с активационным словом
                    current_utterance_buffer = text.split(self.activation_word, 1)[1].strip()
                    if self.ui_linker: self.ui_linker.show_partial_transcription(current_utterance_buffer)
            else:
                # --- СОСТОЯНИЕ: СБОР КОМАНДЫ ---
                if event_type == 'partial':
                    # Обновляем буфер текущей фразы и отображение в UI
                    current_utterance_buffer = text
                    full_display_text = " ".join(command_parts + [current_utterance_buffer]).strip()
                    if self.ui_linker: self.ui_linker.show_partial_transcription(full_display_text)
                
                elif event_type == 'final':
                    # Финальный текст для текущей фразы получен
                    current_utterance_buffer = text
                    
                    if current_utterance_buffer:
                        # Если фраза не пустая, добавляем ее в список частей команды
                        command_parts.append(current_utterance_buffer)
                        current_utterance_buffer = "" # Сбрасываем буфер для следующей фразы
                    
                    # Пустой финальный результат ('') означает паузу в речи - конец команды
                    if not text:
                        if self.ui_linker: self.ui_linker.set_listening_status(False)
                        
                        final_command = " ".join(command_parts).strip()
                        
                        if final_command:
                            print(f"[VoiceController] Распознана полная команда: '{final_command}'")
                            if self.ui_linker: self.ui_linker.last_input_was_voice = True
                            if self.engine: self.engine.submit_task(final_command)
                        else:
                            print("[VoiceController] Активация без команды, сброс.")
                        
                        # Сброс состояния для ожидания следующей команды
                        is_activated = False
                        command_parts = []
                        current_utterance_buffer = ""

    def start_listening(self):
        if not self.is_running_event.is_set():
            self.is_running_event.set()
            if self.listener_thread is None or not self.listener_thread.is_alive():
                self.listener_thread = threading.Thread(target=self._listen_loop)
                self.listener_thread.daemon = True
                self.listener_thread.start()

    def stop_listening(self):
        if self.is_running_event.is_set():
            self.is_running_event.clear()

    def say(self, text: str):
        if not self.tts: return
        self.tts_stop_event.clear()
        with self.tts_queue.mutex: self.tts_queue.queue.clear()
        
        self.tts_queue.put((text, time.time()))