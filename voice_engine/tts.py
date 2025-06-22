# voice_engine/tts.py - Модуль синтеза речи на базе Silero (с обновленным именем модели)

import torch
import silero
import sounddevice as sd
import threading
import os

class TextToSpeech:
    """
    Класс для преобразования текста в речь с использованием Silero Models.
    Поддерживает прерывание (barge-in) через stop_event.
    """
    def __init__(self, model_base_path: str):
        print("[TTS] Загрузка модели Silero...")
        self.device = torch.device('cpu')
        torch.set_num_threads(4)
        
        # --- ИЗМЕНЕНО: Обновлено имя файла на v4_ru.pt, как у вас на скриншоте ---
        local_file = os.path.join(model_base_path, 'v4_ru.pt')
        
        # Указываем Silero, где создавать кеш, если понадобится
        torch.hub.set_dir(os.path.abspath("voice_engine/silero_cache"))

        if not os.path.exists(local_file):
             print(f"[TTS] [ERROR] Не удалось найти локальную модель Silero: {local_file}")
             print(f"[TTS] Пожалуйста, убедитесь, что модель '{os.path.basename(local_file)}' находится в 'voice_engine/silero/'")
             raise FileNotFoundError(local_file)
        
        try:
             self.model = torch.package.PackageImporter(local_file).load_pickle("tts_models", "model")
             self.model.to(self.device)
             print("[TTS] Модель Silero успешно загружена из локального файла.")
        except Exception as e:
             print(f"[TTS] [CRITICAL] Не удалось загрузить пакет модели Silero: {e}")
             raise

        self.sample_rate = 48000
        self.speaker = 'baya' # baya, aidar, kseniya, xenia, eugene

    def speak(self, text: str, stop_event: threading.Event):
        """
        Синтезирует и воспроизводит речь.
        Этот метод предназначен для запуска в отдельном потоке.
        """
        if not text:
            return
            
        print(f"[TTS] Синтезирую речь для: '{text[:30]}...'")
        try:
            # Модель v4 может потребовать немного другую структуру вызова, но apply_tts должен работать
            audio = self.model.apply_tts(
                text=text,
                speaker=self.speaker,
                sample_rate=self.sample_rate
            )
        except Exception as e:
            print(f"[TTS] [ERROR] Ошибка во время синтеза речи: {e}")
            return

        if stop_event.is_set():
            print("[TTS] Воспроизведение отменено (barge-in) еще до начала.")
            return

        print("[TTS] Воспроизвожу речь...")
        
        try:
            stream = sd.OutputStream(
                samplerate=self.sample_rate,
                channels=1,
                dtype='float32'
            )
            stream.start()
            
            buffer_size = 4800 # 100ms
            for i in range(0, len(audio), buffer_size):
                if stop_event.is_set():
                    print("[TTS] Воспроизведение прервано (barge-in).")
                    break
                chunk = audio[i:i+buffer_size]
                stream.write(chunk.numpy())
        except Exception as e:
            print(f"[TTS] [ERROR] Ошибка воспроизведения аудио: {e}")
        finally:
            if 'stream' in locals() and stream.active:
                stream.stop()
                stream.close()