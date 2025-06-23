# voice_engine/stt.py - Исправление пути к модели

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
        # --- ИСПРАВЛЕНО: Убираем лишнее имя папки из пути ---
        # Vosk.Model ожидает путь к папке, содержащей 'am', 'conf' и т.д.
        # а не к родительской папке.
        
        if not os.path.exists(model_path) or not os.listdir(model_path):
             raise FileNotFoundError(f"Директория с моделью Vosk пуста или не найдена по пути: '{model_path}'. "
                                     "Убедитесь, что вы РАСПАКОВАЛИ модель в эту папку.")
        
        print(f"[STT] Загрузка модели Vosk из '{model_path}'...")
        self.model = vosk.Model(model_path)
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
        
        while True:
            try:
                data = stream.read(4096)
                if self.recognizer.AcceptWaveform(data):
                    result_json = self.recognizer.Result()
                    result_dict = json.loads(result_json)
                    text = result_dict.get("text", "")
                    if text:
                        stream.stop_stream()
                        stream.close()
                        return text
            except OSError as e:
                print(f"[STT] Ошибка аудиопотока: {e}. Поток будет перезапущен.")
                stream.stop_stream()
                stream.close()
                import time
                time.sleep(1)
                return self.listen()
            except Exception as e:
                print(f"[STT] Неожиданная ошибка в цикле прослушивания: {e}")
                stream.stop_stream()
                stream.close()
                return ""