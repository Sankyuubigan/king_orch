# voice_engine/tts.py - УБРАН ПРОГРЕВ, ВОЗВРАЩЕНА СТАБИЛЬНАЯ ЛОГИКА

import torch
import silero
import sounddevice as sd
import threading
import os
import time
import traceback # <-- ИМПОРТИРУЕМ ДЛЯ ПОЛНОЙ ДИАГНОСТИКИ

class TextToSpeech:
    def __init__(self, model_base_path: str, speaker: str):
        print("[TTS] Инициализация движка синтеза речи...")
        silero_dir = os.path.join(model_base_path, 'silero')
        local_file = os.path.join(silero_dir, 'v4_ru.pt')
        if not os.path.isfile(local_file):
            raise FileNotFoundError(f"Файл модели Silero не найден по пути: '{local_file}'.")

        self.device = torch.device('cuda' if torch.cuda.is_available() else 'cpu')
        print(f"[TTS] Используется устройство: {self.device}")
        if self.device.type == 'cpu':
            print("[TTS] [WARNING] CUDA не найдена. Синтез речи будет выполняться на CPU, что может быть медленно.")

        torch.set_num_threads(4)
        torch.hub.set_dir(os.path.abspath("voice_engine/silero_cache"))
        
        try:
             print(f"[TTS] Загрузка модели Silero из '{local_file}'...")
             self.model = torch.package.PackageImporter(local_file).load_pickle("tts_models", "model")
             self.model.to(self.device)
             print("[TTS] Модель Silero успешно загружена.")
        except Exception:
             # --- ИСПРАВЛЕНО: ВЫВОДИМ ПОЛНЫЙ ТРЕЙСБЕК В СЛУЧАЕ ОШИБКИ ---
             print(f"[TTS] [FATAL] КРИТИЧЕСКАЯ ОШИБКА ЗАГРУЗКИ МОДЕЛИ SILERO:")
             traceback.print_exc()
             raise

        self.sample_rate = 48000
        self.speaker = speaker
        print(f"[TTS] Выбран диктор: {self.speaker}")
        # --- УДАЛЕНО: Проблемный "прогрев" модели полностью убран. ---

    def speak(self, text: str, stop_event: threading.Event):
        if not text or stop_event.is_set():
            return
        
        print(f"[TTS] Генерация аудио для: '{text}'...")
        try:
            # Простая и надежная генерация всего аудиофайла
            audio = self.model.apply_tts(text=text, speaker=self.speaker, sample_rate=self.sample_rate)
        except Exception:
            print(f"[TTS] [ERROR] Ошибка во время синтеза речи:")
            traceback.print_exc()
            return

        if stop_event.is_set():
            print("[TTS] Воспроизведение отменено (Barge-In).")
            return
        
        stream = None
        try:
            stream = sd.OutputStream(samplerate=self.sample_rate, channels=1, dtype='float32')
            stream.start()
            stream.write(audio.numpy())
            stream.stop()
        except Exception:
            print(f"[TTS] [ERROR] Ошибка воспроизведения аудио:")
            traceback.print_exc()
        finally:
            if stream:
                stream.close()