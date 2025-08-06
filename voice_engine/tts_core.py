# voice_engine/tts_core.py
import os
import sys
import time
import json
from pathlib import Path
import numpy as np
import sounddevice as sd
import threading

# Импортируем ruaccent
try:
    from ruaccent import RUAccent
    RUACCENT_AVAILABLE = True
except ImportError:
    print("Библиотека ruaccent не найдена. Ударения расставляться не будут.", file=sys.stderr)
    RUACCENT_AVAILABLE = False

# Предполагается, что f5_tts установлен и доступен
try:
    from f5_tts.api import F5TTS
    F5TTS_AVAILABLE = True
except ImportError:
    print("Библиотека f5_tts не найдена.", file=sys.stderr)
    F5TTS_AVAILABLE = False

class TTSEngine:
    def __init__(self, model_path_str, voices_config_path, settings_path):
        self.model_path = Path(model_path_str)
        self.voices_config_path = Path(voices_config_path)
        self.settings_path = Path(settings_path)
        self.model = None
        self.voices = []
        self.current_settings = {}
        self.ruaccent_model = None
        self.is_playing = False
        self.stop_playback = threading.Event()
        self.playback_thread = None
        self.load_voices_config()
        self.load_settings()
        self.init_ruaccent()
        self.init_model()

    def init_ruaccent(self):
        """Инициализирует модель ruaccent для расстановки ударений."""
        if RUACCENT_AVAILABLE:
            try:
                print("Загрузка модели ruaccent...")
                self.ruaccent_model = RUAccent()
                # Загружаем модель ударений
                self.ruaccent_model.load(omograph_model_size='big', userdict_model_size='large')
                print("Модель ruaccent загружена.")
            except Exception as e:
                print(f"Ошибка при загрузке модели ruaccent: {e}", file=sys.stderr)
                self.ruaccent_model = None
        else:
            self.ruaccent_model = None

    def add_accent(self, text):
        """Добавляет ударения к тексту с помощью ruaccent."""
        if self.ruaccent_model:
            try:
                accented_text = self.ruaccent_model.process(text)
                print(f"Текст с ударениями: {accented_text}")
                return accented_text
            except Exception as e:
                print(f"Ошибка при обработке ударений: {e}", file=sys.stderr)
                return text
        return text

    def init_model(self):
        """Инициализирует модель F5-TTS."""
        if not F5TTS_AVAILABLE:
            print("Модель F5-TTS недоступна.")
            return

        try:
            print("Загрузка модели F5-TTS...")
            self.model = F5TTS(
                model="F5TTS_v1_Base",
                ckpt_file=str(self.model_path / "model_last_inference.safetensors"),
                device="cuda" if os.environ.get('USE_CUDA', 'false').lower() == 'true' else "cpu",
                vocab_file=str(self.model_path / "vocab.txt"),
            )
            print("Модель F5-TTS загружена.")
        except Exception as e:
            print(f"Ошибка при загрузке модели F5-TTS: {e}", file=sys.stderr)
            self.model = None

    def load_voices_config(self):
        """Загружает конфигурацию голосов."""
        try:
            with open(self.voices_config_path, 'r', encoding='utf-8') as f:
                self.voices = json.load(f)
            print(f"Загружено {len(self.voices)} голосов.")
        except Exception as e:
            print(f"Ошибка при загрузке конфигурации голосов: {e}", file=sys.stderr)
            self.voices = []

    def load_settings(self):
        """Загружает текущие настройки."""
        try:
            with open(self.settings_path, 'r', encoding='utf-8') as f:
                self.current_settings = json.load(f)
        except FileNotFoundError:
            self.current_settings = {"selected_voice_index": 0}
            self.save_settings()
        except Exception as e:
            print(f"Ошибка при загрузке настроек: {e}", file=sys.stderr)
            self.current_settings = {"selected_voice_index": 0}

    def save_settings(self):
        """Сохраняет текущие настройки."""
        try:
            with open(self.settings_path, 'w', encoding='utf-8') as f:
                json.dump(self.current_settings, f, indent=4, ensure_ascii=False)
        except Exception as e:
            print(f"Ошибка при сохранении настроек: {e}", file=sys.stderr)

    def get_voice_list(self):
        """Возвращает список имен голосов для отображения в UI."""
        return [voice['name'] for voice in self.voices]

    def set_selected_voice(self, index):
        """Устанавливает выбранный голос по индексу."""
        if 0 <= index < len(self.voices):
            self.current_settings['selected_voice_index'] = index
            self.save_settings()
            print(f"Выбран голос: {self.voices[index]['name']}")
        else:
            print(f"Неверный индекс голоса: {index}", file=sys.stderr)

    def get_selected_voice(self):
        """Возвращает информацию о текущем выбранном голосе."""
        index = self.current_settings.get('selected_voice_index', 0)
        if 0 <= index < len(self.voices):
            voice_info = self.voices[index].copy()
            voice_info['full_path'] = str(self.voices_config_path.parent / voice_info['file'])
            return voice_info
        else:
            return None

    # --- Логика воспроизведения (взята из предыдущего рабочего варианта) ---
    def _audio_callback(self, outdata, frames, time_info, status):
        """Колбэк для воспроизведения аудио."""
        # Предполагается, что audio_data и stop_event доступны из внешнего scope
        # или передаются иначе. Для упрощения сделаем их атрибутами экземпляра
        # во время воспроизведения.
        if status:
            print(f"Статус аудио: {status}", file=sys.stderr)

        if self.stop_playback.is_set() or not hasattr(self, '_audio_data') or len(self._audio_data) == 0:
            raise sd.CallbackStop()

        n_remaining = len(self._audio_data)
        n_requested = len(outdata)

        if n_remaining >= n_requested:
            outdata[:] = self._audio_data[:n_requested].reshape(-1, 1)
            self._audio_data = self._audio_data[n_requested:]
        else:
            outdata[:n_remaining] = self._audio_data.reshape(-1, 1)
            outdata[n_remaining:] = 0
            self._audio_data = np.array([])
            self.stop_playback.set()

    def _play_audio_thread(self, wav, sr):
        """Функция для воспроизведения аудио в отдельном потоке."""
        self.is_playing = True
        self.stop_playback.clear()

        # Нормализуем аудио
        if wav.dtype != np.float32:
            if np.issubdtype(wav.dtype, np.integer):
                wav_normalized = wav.astype(np.float32) / np.max(np.abs(wav))
            else:
                wav_normalized = wav.astype(np.float32)
        else:
            wav_normalized = wav

        # Подготовка данных для колбэка
        self._audio_data = wav_normalized

        try:
            stream = sd.OutputStream(
                samplerate=sr,
                channels=1,
                callback=self._audio_callback,
                dtype='float32',
                latency='high'
            )
            with stream:
                while not self.stop_playback.is_set():
                    sd.sleep(int(1000 * 0.05)) # 50ms sleep
        except Exception as e:
            print(f"Ошибка при воспроизведении: {e}", file=sys.stderr)
        finally:
            self.is_playing = False
            if hasattr(self, '_audio_data'):
                del self._audio_data

    def stop_playback_now(self):
        """Останавливает текущее воспроизведение."""
        if self.is_playing and self.playback_thread and self.playback_thread.is_alive():
            self.stop_playback.set()
            # Даем потоку немного времени завершиться
            self.playback_thread.join(timeout=1.0)
            self.is_playing = False
            print("Воспроизведение остановлено.")

    def synthesize_and_play(self, text):
        """Синтезирует аудио из текста и воспроизводит его."""
        # Останавливаем предыдущее воспроизведение
        self.stop_playback_now()

        if not self.model:
            print("Модель не загружена.")
            return False

        selected_voice = self.get_selected_voice()
        if not selected_voice:
            print("Голос не выбран или конфигурация некорректна.")
            return False

        ref_file = selected_voice['full_path']
        ref_text = selected_voice['ref_text']

        if not os.path.exists(ref_file):
            print(f"Файл референсного аудио не найден: {ref_file}")
            return False

        accented_text = self.add_accent(text)

        print(f"Генерация аудио для текста: {accented_text}")
        print(f"Используется голос: {selected_voice['name']} ({ref_file})")

        try:
            start_time = time.time()
            wav, sr, _ = self.model.infer(
                ref_file=ref_file,
                ref_text=ref_text,
                gen_text=accented_text,
                file_wave=None,
                remove_silence=True,
                seed=42
            )
            gen_time = time.time() - start_time
            print(f"Аудио сгенерировано за {gen_time:.2f} секунд")

            # Запускаем воспроизведение в новом потоке
            self.playback_thread = threading.Thread(target=self._play_audio_thread, args=(wav, sr), daemon=True)
            self.playback_thread.start()
            return True
        except Exception as e:
            print(f"Ошибка при синтезе речи: {e}", file=sys.stderr)
            return False

# --- Пример использования ---
# if __name__ == "__main__":
#     MODEL_PATH = r"D:\nn\models\tts\Misha2410-F5-TTS_RUSSIAN"
#     VOICES_CONFIG_PATH = "voice_engine/voices/voices.json"
#     SETTINGS_PATH = "voice_engine/settings.json"
#
#     tts_engine = TTSEngine(MODEL_PATH, VOICES_CONFIG_PATH, SETTINGS_PATH)
#     tts_engine.synthesize_and_play("Привет, это тест синтеза речи с автоматической расстановкой ударений.")
#     # Ждем немного, чтобы услышать
#     import time
#     time.sleep(5)