# voice_engine/tts.py - ИСПРАВЛЕНА ОШИБКА ValueError И УЛУЧШЕНА ЛОГИКА ЗАГРУЗКИ

import torch
import silero
import sounddevice as sd
import threading
import os
import time
import traceback

class TextToSpeech:
    def __init__(self, model_base_path: str, speaker: str):
        print("[TTS] Инициализация движка синтеза речи...")
        
        self.device = torch.device('cuda' if torch.cuda.is_available() else 'cpu')
        print(f"[TTS] Используется устройство: {self.device}")
        if self.device.type == 'cpu':
            print("[TTS] [WARNING] CUDA не найдена. Убедитесь, что установлена версия PyTorch с поддержкой CUDA. Синтез будет медленным.")

        torch.set_num_threads(4)
        # Устанавливаем кэш в предсказуемое место, чтобы избежать повторных загрузок
        torch.hub.set_dir(os.path.abspath("voice_engine/silero_cache"))
        
        try:
             print(f"[TTS] Загрузка модели Silero и утилит...")
             # --- ИСПРАВЛЕНИЕ: ПРАВИЛЬНО ЗАГРУЖАЕМ И МОДЕЛЬ, И УТИЛИТЫ ---
             # torch.hub.load возвращает кортеж (модель, утилиты), а не только модель.
             self.model, self.utils = torch.hub.load(repo_or_dir='snakers4/silero-models',
                                                     model='silero_tts',
                                                     language='ru',
                                                     speaker='v4_ru',
                                                     trust_repo=True) # Доверяем репозиторию, чтобы избежать запросов в консоли
             self.model.to(self.device)
             print("[TTS] Модель Silero успешно загружена.")
        except Exception:
             print(f"[TTS] [FATAL] КРИТИЧЕСКАЯ ОШИБКА ЗАГРУЗКИ МОДЕЛИ SILERO:")
             traceback.print_exc()
             raise

        self.sample_rate = 48000
        self.speaker = speaker
        print(f"[TTS] Выбран диктор: {self.speaker}")

    def speak(self, text: str, stop_event: threading.Event):
        if not text or stop_event.is_set():
            return
        
        total_start_time = time.time()
        print(f"[TTS] Поступил запрос на озвучку: '{text[:50]}...'")
        
        # --- ИСПРАВЛЕНИЕ: ПРАВИЛЬНО ПОЛУЧАЕМ ФУНКЦИЮ ДЛЯ РАЗБИВКИ ---
        # Функция является частью объекта utils, а не отдельной переменной.
        split_into_sentences = self.utils.split_into_sentences
        sentences = split_into_sentences(text, 'ru')
        
        stream = None
        first_chunk_played = False
        
        try:
            stream = sd.OutputStream(samplerate=self.sample_rate, channels=1, dtype='float32')
            stream.start()
            
            for i, sentence in enumerate(sentences):
                if stop_event.is_set():
                    print("[TTS] Воспроизведение прервано (Barge-In).")
                    break
                
                if not sentence.strip(): continue

                gen_start_time = time.time()
                print(f"  [TTS] -> Генерация аудио для предложения #{i+1}...")
                
                audio_chunk = self.model.apply_tts(text=sentence, speaker=self.speaker, sample_rate=self.sample_rate)
                
                gen_end_time = time.time()
                print(f"  [TTS] <- Генерация заняла {gen_end_time - gen_start_time:.2f} сек.")
                
                if stop_event.is_set():
                    print("[TTS] Воспроизведение прервано после генерации (Barge-In).")
                    break

                stream.write(audio_chunk.numpy())
                
                if not first_chunk_played:
                    first_chunk_time = time.time()
                    print(f"  [TTS] !!! Первый кусок аудио начал воспроизводиться через {first_chunk_time - total_start_time:.2f} сек. !!!")
                    first_chunk_played = True

            stream.stop()
            
        except Exception:
            print(f"[TTS] [ERROR] Ошибка во время потокового синтеза или воспроизведения:")
            traceback.print_exc()
        finally:
            if stream:
                stream.close()
        
        total_end_time = time.time()
        print(f"[TTS] Полная обработка фразы завершена за {total_end_time - total_start_time:.2f} сек.")