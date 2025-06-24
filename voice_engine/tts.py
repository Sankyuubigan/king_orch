# voice_engine/tts.py - УНИВЕРСАЛЬНЫЙ TTS-ДВИЖОК С ПОДДЕРЖКОЙ 3 МОДЕЛЕЙ

import torch
import sounddevice as sd
import threading
import os
import time
import re
from huggingface_hub import hf_hub_download

class TextToSpeech:
    def __init__(self, engine_id: str = "silero", device: str = "cpu", settings: dict = {}):
        print(f"[TTS Factory] Инициализация движка '{engine_id}' на устройстве '{device}'...")
        self.engine_id = engine_id
        self.settings = settings
        self.engine = None
        self.is_ready = False
        
        if device == 'cuda' and not torch.cuda.is_available():
            print("[TTS Factory] [WARNING] CUDA не найдена, используется CPU.")
            self.device = torch.device('cpu')
        else:
            self.device = torch.device(device)
            
        self._load_engine()

    def _load_engine(self):
        """Фабрика-загрузчик для выбора и инициализации TTS-модели."""
        try:
            if self.engine_id == "silero": self._load_silero()
            elif self.engine_id == "xtts": self._load_xtts()
            elif self.engine_id == "f5": self._load_f5()
            else: print(f"[TTS Factory] [ERROR] Неизвестный движок TTS: {self.engine_id}")
            self.is_ready = True
        except Exception as e:
            print(f"[TTS Factory] [FATAL] Не удалось загрузить движок '{self.engine_id}': {e}", exc_info=True)
            self.is_ready = False

    def speak(self, text: str, stop_event: threading.Event):
        """Универсальный метод для синтеза речи."""
        if not text or stop_event.is_set() or not self.is_ready: return
        print(f"[{self.engine_id.upper()}]-TTS] Запрос на озвучку: '{text[:40]}...'")
        
        try:
            # Для каждого движка своя логика вызова
            if self.engine_id == "silero":
                audio = self.engine.apply_tts(text=text, speaker=self.settings.get("tts_silero_speaker", "aidar"), sample_rate=48000)
                sd.play(audio, samplerate=48000)
            
            elif self.engine_id == "xtts":
                # XTTS генерирует WAV, который мы проигрываем.
                # TODO: Добавить опцию клонирования голоса speaker_wav='path/to/voice.wav'
                wav = self.engine.tts(text=text, speaker=self.engine.speakers[0], language=self.engine.languages[0])
                sd.play(wav, samplerate=24000)
            
            elif self.engine_id == "f5":
                accent_engine, model = self.engine
                text_with_stress = accent_engine.process_text(text)
                inputs = self.tokenizer(text_with_stress, return_tensors="pt").to(self.device)
                with torch.no_grad():
                    output = model.generate(**inputs, max_length=len(text_with_stress)*10)
                audio = output[0].cpu().numpy().squeeze()
                sd.play(audio, samplerate=model.config.sampling_rate)
            
            sd.wait() # Ждем окончания воспроизведения
        except Exception as e:
            print(f"[{self.engine_id.upper()}-TTS] [ERROR] Ошибка во время синтеза речи: {e}")

    def _load_silero(self):
        """Загрузка модели Silero."""
        model_file = os.path.join("voice_engine", "tts", "silero_v4_ru.pt")
        if not os.path.isfile(model_file):
            torch.hub.download_url_to_file('https://models.silero.ai/models/tts/ru/v4_ru.pt', model_file)
        self.engine = torch.package.PackageImporter(model_file).load_pickle("tts_models", "model")
        self.engine.to(self.device)
        torch.set_num_threads(4)
        print("[TTS Factory] Silero загружен.")

    def _load_xtts(self):
        """Загрузка модели XTTSv2."""
        from TTS.api import TTS
        self.engine = TTS("tts_models/multilingual/multi-dataset/xtts_v2", progress_bar=True).to(self.device)
        print("[TTS Factory] XTTS-v2 загружен.")
    
    def _load_f5(self):
        """Загрузка модели F5-TTS."""
        from transformers import F5ForSpeech, F5Tokenizer
        from ruaccent import RuAccent
        
        repo_id = "Misha24-10/F5-TTS_RUSSIAN"
        model_filename = "F5TTS_v1_Base/model_240000_inference.safetensors"
        vocab_filename = "F5TTS_v1_Base/vocab.txt"
        
        # Скачиваем файлы, если их нет
        model_path = hf_hub_download(repo_id=repo_id, filename=model_filename)
        vocab_path = hf_hub_download(repo_id=repo_id, filename=vocab_filename)
        
        # Инициализация
        tokenizer = F5Tokenizer(vocab_path)
        model = F5ForSpeech.from_pretrained(model_path, low_cpu_mem_usage=True).to(self.device)
        accent_engine = RuAccent()
        accent_engine.load(omograph_model_size='big', use_dictionary=True)

        self.tokenizer = tokenizer # Сохраняем токенайзер отдельно
        self.engine = (accent_engine, model) # Сохраняем кортеж из двух компонентов
        print("[TTS Factory] F5-TTS загружен.")