import torch
import sounddevice as sd
import threading
import os
import time
import traceback
import logging
from huggingface_hub import hf_hub_download

logger = logging.getLogger(__name__)

# Минимально необходимая версия transformers для F5-TTS
MIN_TRANSFORMERS_VERSION = "4.36.0"

class TextToSpeech:
    def __init__(self, engine_id: str = "silero", device: str = "cpu", settings: dict = {}):
        logger.info(f"[TTS Factory] Инициализация движка '{engine_id}' на устройстве '{device}'...")
        self.engine_id = engine_id
        self.settings = settings
        self.engine = None
        self.is_ready = False
        
        if device == 'cuda' and not torch.cuda.is_available():
            logger.warning("[TTS Factory] CUDA не найдена, используется CPU.")
            self.device = torch.device('cpu')
        else:
            self.device = torch.device(device)
            
        self._load_engine()

    def _load_engine(self):
        """Фабрика-загрузчик для выбора и инициализации TTS-модели."""
        try:
            if self.engine_id == "silero": self._load_silero()
            elif self.engine_id == "f5": self._load_f5()
            else: logger.error(f"[TTS Factory] Неизвестный или отключенный движок TTS: {self.engine_id}")
            
            if self.engine:
                self.is_ready = True
        except Exception as e:
            logger.critical(f"[TTS Factory] [FATAL] Не удалось загрузить движок '{self.engine_id}': {e}\n{traceback.format_exc()}")
            self.is_ready = False

    def speak(self, text: str, stop_event: threading.Event):
        """Универсальный метод для синтеза речи."""
        if not text or stop_event.is_set() or not self.is_ready: return
        logger.info(f"[{self.engine_id.upper()}-TTS] Запрос на озвучку: '{text[:40]}...'")
        
        try:
            if self.engine_id == "silero":
                audio = self.engine.apply_tts(text=text, speaker=self.settings.get("tts_silero_speaker", "aidar"), sample_rate=48000)
                sd.play(audio, samplerate=48000)
            
            elif self.engine_id == "f5":
                accent_engine, model, tokenizer = self.engine
                text_with_stress = accent_engine.process_text(text)
                inputs = tokenizer(text_with_stress, return_tensors="pt").to(self.device)
                with torch.no_grad():
                    output = model.generate(**inputs, max_length=len(text_with_stress)*10)
                audio = output.cpu().numpy().squeeze()
                sd.play(audio, samplerate=model.config.sampling_rate)
            
            sd.wait()
        except Exception as e:
            logger.error(f"[{self.engine_id.upper()}-TTS] [ERROR] Ошибка во время синтеза речи: {e}")

    def _load_silero(self):
        """Загрузка модели Silero."""
        model_file = os.path.join("voice_engine", "tts", "silero_v4_ru.pt")
        if not os.path.isfile(model_file):
            torch.hub.download_url_to_file('https://models.silero.ai/models/tts/ru/v4_ru.pt', model_file)
        self.engine = torch.package.PackageImporter(model_file).load_pickle("tts_models", "model")
        self.engine.to(self.device)
        torch.set_num_threads(4)
        logger.info("[TTS Factory] Silero загружен.")

    def _load_f5(self):
        """Загрузка модели F5-TTS с явной проверкой версии."""
        try:
            # ИСПРАВЛЕНИЕ: Добавляем явную проверку версии с помощью pkg_resources
            from pkg_resources import parse_version
            import transformers
            if parse_version(transformers.__version__) < parse_version(MIN_TRANSFORMERS_VERSION):
                logger.error("="*80)
                logger.error(f"[TTS Factory] [FATAL] Версия библиотеки 'transformers' ({transformers.__version__}) устарела.")
                logger.error(f"Для работы движка F5-TTS требуется версия >= {MIN_TRANSFORMERS_VERSION}.")
                logger.error("Пожалуйста, выполните в терминале команду: pip install -r requirements.txt --upgrade")
                logger.error("="*80)
                self.engine = None
                return

            from transformers import F5ForSpeech, F5Tokenizer
            from ruaccent import RuAccent
        except ImportError:
            logger.error("[TTS Factory] [ERROR] Не удалось импортировать компоненты. Убедитесь, что все зависимости установлены.")
            logger.error("Движок F5-TTS будет отключен.")
            self.engine = None
            return

        repo_id = "Misha24-10/F5-TTS_RUSSIAN"
        model_filename = "F5TTS_v1_Base/model_240000_inference.safetensors"
        vocab_filename = "F5TTS_v1_Base/vocab.txt"
        
        model_path = hf_hub_download(repo_id=repo_id, filename=model_filename)
        vocab_path = hf_hub_download(repo_id=repo_id, filename=vocab_filename)
        
        tokenizer = F5Tokenizer(vocab_path)
        model = F5ForSpeech.from_pretrained(model_path, low_cpu_mem_usage=True).to(self.device)
        accent_engine = RuAccent()
        accent_engine.load(omograph_model_size='big', use_dictionary=True)

        self.engine = (accent_engine, model, tokenizer)
        logger.info("[TTS Factory] F5-TTS загружен.")