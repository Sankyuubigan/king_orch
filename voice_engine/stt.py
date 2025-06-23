# voice_engine/stt.py - Модифицирован для потоковой передачи частичных результатов

import vosk
import pyaudio
import json
import os
import time

class SpeechToText:
    def __init__(self, model_path: str):
        print(f"[STT] Попытка загрузки модели Vosk из '{model_path}'...")
        try:
            self.model = vosk.Model(model_path)
        except Exception as e:
            print(f"\n!!! [STT] КРИТИЧЕСКАЯ ОШИБКА: Библиотека Vosk не смогла загрузить модель: {e} !!!\n")
            raise
            
        self.recognizer = vosk.KaldiRecognizer(self.model, 16000)
        self.recognizer.SetWords(True) # Включаем режим для получения частичных результатов
        self.audio_interface = pyaudio.PyAudio()
        print("[STT] Модель Vosk и аудиоинтерфейс успешно загружены.")

    def listen(self):
        """
        Открывает аудиопоток и слушает, возвращая (yield) промежуточные
        и финальные результаты распознавания.
        """
        stream = None
        try:
            stream = self.audio_interface.open(
                format=pyaudio.paInt16,
                channels=1,
                rate=16000,
                input=True,
                frames_per_buffer=4096 # Уменьшаем буфер для меньшей задержки
            )
            
            while True:
                data = stream.read(2048, exception_on_overflow=False)
                
                if self.recognizer.AcceptWaveform(data):
                    # Распознана финальная фраза
                    result_json = json.loads(self.recognizer.Result())
                    final_text = result_json.get("text", "")
                    if final_text:
                        yield 'final', final_text
                else:
                    # Получаем промежуточный результат
                    partial_json = json.loads(self.recognizer.PartialResult())
                    partial_text = partial_json.get("partial", "")
                    if partial_text:
                        yield 'partial', partial_text

        except OSError as e:
            print(f"[STT] Ошибка аудиопотока: {e}. Поток будет перезапущен.")
            time.sleep(1)
            # Рекурсивно перезапускаем генератор
            yield from self.listen()
        except Exception as e:
            print(f"[STT] Неожиданная ошибка в цикле прослушивания: {e}")
            yield 'error', str(e)
        finally:
            if stream:
                stream.stop_stream()
                stream.close()