# voice_engine/stt.py - Модуль распознавания речи на базе Vosk

import vosk
import pyaudio
import json
import os

class SpeechToText:
    """
    Класс для преобразования речи в текст с использованием Vosk.
    Работает в блокирующем режиме, возвращая распознанный текст.
    """
    def __init__(self, model_path: str):
        # Проверяем корректность пути к модели, как у вас на скриншоте
        full_model_path = os.path.join(model_path, "vosk-model-tts-ru-0.8-multi")

        if not os.path.exists(full_model_path):
            raise FileNotFoundError(f"Модель Vosk не найдена по пути: {full_model_path}. "
                                    "Убедитесь, что вы распаковали модель в нужную директорию.")
        
        print(f"[STT] Загрузка модели Vosk из '{full_model_path}'...")
        self.model = vosk.Model(full_model_path)
        self.recognizer = vosk.KaldiRecognizer(self.model, 16000)
        self.audio_interface = pyaudio.PyAudio()
        print("[STT] Модель Vosk успешно загружена.")

    def listen(self) -> str:
        """
        Открывает аудиопоток и слушает до тех пор, пока не будет распознана
        законченная фраза. Возвращает эту фразу в виде текста.
        """
        stream = self.audio_interface.open(
            format=pyaudio.paInt16,
            channels=1,
            rate=16000,
            input=True,
            frames_per_buffer=8192
        )
        
        # print("[STT] Начал слушать...")
        while True:
            try:
                data = stream.read(4096)
                if self.recognizer.AcceptWaveform(data):
                    result_json = self.recognizer.Result()
                    result_dict = json.loads(result_json)
                    text = result_dict.get("text", "")
                    if text:
                        # print(f"[STT] Распознано: '{text}'")
                        stream.stop_stream()
                        stream.close()
                        return text
            except OSError as e:
                # Эта ошибка часто возникает в Windows при переключении аудиоустройств
                print(f"[STT] Ошибка аудиопотока: {e}. Поток будет перезапущен.")
                stream.stop_stream()
                stream.close()
                # Даем системе секунду на восстановление
                import time
                time.sleep(1)
                return self.listen() # Рекурсивный вызов для переоткрытия потока
            except Exception as e:
                print(f"[STT] Неожиданная ошибка в цикле прослушивания: {e}")
                stream.stop_stream()
                stream.close()
                return ""