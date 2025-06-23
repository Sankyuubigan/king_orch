# voice_engine/tts.py - ВОЗВРАЩЕН К ИСХОДНОЙ ВЕРСИИ + 1 СТРОКА ДЛЯ ОТКЛЮЧЕНИЯ ПРОФАЙЛЕРА

import torch
import sounddevice as sd
import threading
import os
import time
import traceback
import re

# --- РЕШЕНИЕ: Отключаем JIT-профайлер, чтобы убрать задержку первого запуска ---
torch._C._jit_set_profiling_mode(False)

class TextToSpeech:
    def __init__(self, model_base_path: str, speaker: str, device: str):
        print("[TTS] Инициализация движка синтеза речи...")
        
        if device == 'cuda' and not torch.cuda.is_available():
            print("[TTS] [WARNING] CUDA выбрана, но недоступна. Переключаюсь на CPU.")
            self.device = torch.device('cpu')
        else:
            self.device = torch.device(device)

        print(f"[TTS] Используется устройство: {self.device}")
        
        torch.set_num_threads(8)
        
        try:
            model_file = os.path.join(model_base_path, 'silero', 'v4_ru.pt')
            
            if not os.path.isfile(model_file):
                print(f"[TTS] Файл модели не найден. Загрузка в '{model_file}'...")
                os.makedirs(os.path.dirname(model_file), exist_ok=True)
                torch.hub.download_url_to_file('https://models.silero.ai/models/tts/ru/v4_ru.pt', model_file)
            
            print(f"[TTS] Загрузка модели из локального файла: {model_file}")
            self.model = torch.package.PackageImporter(model_file).load_pickle("tts_models", "model")
            self.model.to(self.device)
            print("[TTS] Модель Silero успешно загружена.")

        except Exception:
             print(f"[TTS] [FATAL] КРИТИЧЕСКАЯ ОШИБКА ЗАГРУЗКИ МОДЕЛИ SILERO:")
             traceback.print_exc()
             raise

        self.sample_rate = 48000
        self.speaker = speaker
        print(f"[TTS] Выбран диктор: {self.speaker}")

    def speak(self, text: str, stop_event: threading.Event, is_warm_up: bool = False):
        if not text or stop_event.is_set():
            return
        
        if is_warm_up:
            print(f"[TTS speak] Получена команда на прогрев.")
        else:
            print(f"[TTS speak] Поступил запрос на озвучку: '{text[:50]}...'")
        
        sentences = re.split(r'(?<=[.!?])\s+', text.strip())
        
        stream = None
        
        try:
            # При прогреве поток не создается, чтобы не было звука
            if not is_warm_up:
                stream = sd.OutputStream(samplerate=self.sample_rate, channels=1, dtype='float32')
                stream.start()

            for i, sentence in enumerate(sentences):
                if stop_event.is_set():
                    print("[TTS speak] Воспроизведение прервано (Barge-In).")
                    break
                
                if not sentence.strip(): continue

                if self.device.type == 'cuda':
                    torch.cuda.synchronize()
                
                gen_start_time = time.time()
                
                audio_chunk = self.model.apply_tts(text=sentence,
                                                   speaker=self.speaker,
                                                   sample_rate=self.sample_rate)
                
                if self.device.type == 'cuda':
                    torch.cuda.synchronize()
                
                gen_end_time = time.time()

                print(f"  [TTS speak] <- Генерация на {self.device.type.upper()} заняла {gen_end_time - gen_start_time:.2f} сек.")
                
                if stop_event.is_set():
                    break

                if not is_warm_up and stream:
                    stream.write(audio_chunk.numpy())

            if not is_warm_up and stream:
                stream.stop()
            
        except Exception:
            print(f"[TTS speak] [ERROR] Ошибка во время потокового синтеза или воспроизведения:")
            traceback.print_exc()
        finally:
            if not is_warm_up and stream:
                stream.close()