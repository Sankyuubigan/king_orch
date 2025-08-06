# tests\onnx_tts_engine.py
import os
import json
import time
import numpy as np
from pathlib import Path
import onnxruntime as ort
import soundfile as sf
import librosa
import signal
import sys

# --- Импорт ruaccent ---
RUACCENT_AVAILABLE = False
try:
    from ruaccent import RUAccent
    RUACCENT_AVAILABLE = True
    print("Библиотека ruaccent найдена.")
except ImportError:
    print("Библиотека ruaccent не найдена. Ударения расставляться не будут.")
except KeyboardInterrupt:
    print("\nЗагрузка ruaccent прервана пользователем.")
    sys.exit(0)
except Exception as e:
    print(f"Неожиданная ошибка при импорте ruaccent: {e}")

class ONNXTTSEngine:
    def __init__(self, model_a_path, model_b_path, model_c_path, vocab_path, voices_config_path, settings_path):
        self.model_a_path = Path(model_a_path)
        self.model_b_path = Path(model_b_path)
        self.model_c_path = Path(model_c_path)
        self.vocab_path = Path(vocab_path)
        self.voices_config_path = Path(voices_config_path)
        self.settings_path = Path(settings_path)

        missing_files = []
        for name, path in [("vocab.txt", self.vocab_path), ("voices.json", self.voices_config_path),
                           ("F5_Preprocess.onnx", self.model_a_path), ("F5_Transformer.onnx", self.model_b_path),
                           ("F5_Decode.onnx", self.model_c_path)]:
            if not path.exists():
                missing_files.append(f"{name}: {path}")

        if missing_files:
            raise FileNotFoundError(f"Не найдены необходимые файлы:\n" + "\n".join(missing_files))

        self.vocab_char_map = self._load_vocab()
        self.vocab_size = len(self.vocab_char_map)

        self.voices = self._load_voices_config()
        self.current_settings = self._load_settings()

        self.ruaccent_model = None
        self._init_ruaccent()

        # Параметры модели (должны совпадать с параметрами экспорта)
        self.SAMPLE_RATE = 24000
        self.HOP_LENGTH = 256
        self.N_MELS = 100
        self.NFFT = 1024
        self.WINDOW_LENGTH = 960
        self.WINDOW_TYPE = 'hann'
        self.AUDIO_LENGTH = 160000
        self.TEXT_IDS_LENGTH = 60
        self.MAX_GENERATED_LENGTH = 600
        self.TEXT_EMBED_LENGTH = 512 + self.N_MELS
        self.REFERENCE_SIGNAL_LENGTH = self.AUDIO_LENGTH // self.HOP_LENGTH + 1
        self.MAX_SIGNAL_LENGTH = 4096
        self.MAX_DURATION = self.REFERENCE_SIGNAL_LENGTH + self.MAX_GENERATED_LENGTH
        if self.MAX_DURATION > self.MAX_SIGNAL_LENGTH:
            self.MAX_DURATION = self.MAX_SIGNAL_LENGTH

        self.NFE_STEP = 32
        self.FUSE_NFE = 1
        self.CFG_STRENGTH = 2.0
        self.SWAY_COEFFICIENT = -1.0
        self.TARGET_RMS = 0.15
        self.SPEED = 1.0

        print("Загрузка ONNX моделей...")
        self.ort_session_A = None
        self.ort_session_B = None
        self.ort_session_C = None
        self._init_onnx_sessions()
        print("ONNX модели загружены.")

    def _load_vocab(self):
        vocab_char_map = {}
        try:
            with open(self.vocab_path, "r", encoding="utf-8") as f:
                for i, char in enumerate(f):
                    vocab_char_map[char[:-1]] = i
            print(f"Словарь загружен. Размер: {len(vocab_char_map)}")
        except Exception as e:
            print(f"Ошибка при загрузке словаря '{self.vocab_path}': {e}")
            raise e
        return vocab_char_map

    def _load_voices_config(self):
        try:
            with open(self.voices_config_path, 'r', encoding='utf-8') as f:
                voices = json.load(f)
            print(f"Конфигурация голосов загружена. Кол-во голосов: {len(voices)}")
            return voices
        except Exception as e:
            print(f"Ошибка при загрузке конфигурации голосов '{self.voices_config_path}': {e}")
            raise e

    def _load_settings(self):
        try:
            with open(self.settings_path, 'r', encoding='utf-8') as f:
                settings = json.load(f)
            return settings
        except FileNotFoundError:
            default_settings = {"selected_voice_index": 0}
            self._save_settings(default_settings)
            return default_settings
        except Exception as e:
            print(f"Ошибка при загрузке настроек '{self.settings_path}': {e}")
            return {"selected_voice_index": 0}

    def _save_settings(self, settings):
        try:
            os.makedirs(os.path.dirname(self.settings_path), exist_ok=True)
            with open(self.settings_path, 'w', encoding='utf-8') as f:
                json.dump(settings, f, indent=4, ensure_ascii=False)
        except Exception as e:
            print(f"Ошибка при сохранении настроек: {e}")

    def _init_ruaccent(self):
        if RUACCENT_AVAILABLE:
            try:
                print("Загрузка модели ruaccent...")
                self.ruaccent_model = RUAccent()
                # Попробуем разные комбинации аргументов
                try:
                    self.ruaccent_model.load()
                    print("Модель ruaccent загружена (без аргументов).")
                except Exception:
                    try:
                        self.ruaccent_model.load(omograph_model_size='big')
                        print("Модель ruaccent загружена (omograph_model_size='big').")
                    except Exception:
                        try:
                            self.ruaccent_model.load(omograph_model_size='big', use_dictionary=False)
                            print("Модель ruaccent загружена (omograph_model_size='big', use_dictionary=False).")
                        except Exception as e:
                            print(f"Не удалось загрузить модель ruaccent с известными аргументами: {e}")
                            self.ruaccent_model = None
            except Exception as e:
                print(f"Ошибка при инициализации модели ruaccent: {e}")
                self.ruaccent_model = None
        else:
            self.ruaccent_model = None

    def _init_onnx_sessions(self):
        session_opts = ort.SessionOptions()
        session_opts.log_severity_level = 4
        session_opts.log_verbosity_level = 4
        session_opts.inter_op_num_threads = 0
        session_opts.intra_op_num_threads = 0
        session_opts.enable_cpu_mem_arena = True
        session_opts.execution_mode = ort.ExecutionMode.ORT_SEQUENTIAL
        session_opts.graph_optimization_level = ort.GraphOptimizationLevel.ORT_ENABLE_ALL
        session_opts.add_session_config_entry("session.intra_op.allow_spinning", "1")
        session_opts.add_session_config_entry("session.inter_op.allow_spinning", "1")
        session_opts.add_session_config_entry("session.set_denormal_as_zero", "1")

        try:
            self.ort_session_A = ort.InferenceSession(str(self.model_a_path), sess_options=session_opts, providers=['CPUExecutionProvider'])

            available_providers = ort.get_available_providers()
            print(f"Доступные провайдеры ONNX Runtime: {available_providers}")
            transformer_provider = ['CPUExecutionProvider']
            if 'CUDAExecutionProvider' in available_providers:
                transformer_provider = ['CUDAExecutionProvider']
                print("ONNX Transformer будет использовать CUDA.")
            elif 'DmlExecutionProvider' in available_providers:
                 transformer_provider = ['DmlExecutionProvider']
                 print("ONNX Transformer будет использовать DirectML.")
            else:
                print("ONNX Transformer будет использовать CPU.")

            self.ort_session_B = ort.InferenceSession(str(self.model_b_path), sess_options=session_opts, providers=transformer_provider)
            self.ort_session_C = ort.InferenceSession(str(self.model_c_path), sess_options=session_opts, providers=['CPUExecutionProvider'])
        except Exception as e:
            print(f"Ошибка при инициализации ONNX сессий: {e}")
            raise e

        try:
            self.in_name_A = [inp.name for inp in self.ort_session_A.get_inputs()]
            self.out_name_A = [out.name for out in self.ort_session_A.get_outputs()]
            self.in_name_B = [inp.name for inp in self.ort_session_B.get_inputs()]
            self.out_name_B = [out.name for out in self.ort_session_B.get_outputs()]
            self.in_name_C = [inp.name for inp in self.ort_session_C.get_inputs()]
            self.out_name_C = [out.name for out in self.ort_session_C.get_outputs()]
            print("Входы/выходы ONNX моделей определены.")
        except Exception as e:
            print(f"Ошибка при получении входов/выходов ONNX моделей: {e}")
            raise e

    # --- ИСПРАВЛЕНИЕ: Обновленный метод add_accent ---
    def add_accent(self, text):
        """Добавляет ударения к тексту с помощью ruaccent."""
        if self.ruaccent_model:
            try:
                # ИСПРАВЛЕНИЕ: Используем правильный метод process_all
                accented_text = self.ruaccent_model.process_all(text)
                print(f"Текст с ударениями: {accented_text}")
                return accented_text
            except AttributeError:
                print("Метод 'process_all' не найден в объекте ruaccent. Проверьте версию библиотеки.")
            except Exception as e:
                print(f"Ошибка при обработке ударений: {e}")
        return text
    # --- Конец исправления ---

    def get_voice_list(self):
        """Возвращает список имен голосов."""
        return [voice['name'] for voice in self.voices]

    def set_selected_voice(self, index):
        """Устанавливает выбранный голос."""
        if 0 <= index < len(self.voices):
            self.current_settings['selected_voice_index'] = index
            self._save_settings(self.current_settings)
            print(f"Выбран голос: {self.voices[index]['name']}")

    def get_selected_voice(self):
        """Возвращает информацию о текущем выбранном голосе."""
        index = self.current_settings.get('selected_voice_index', 0)
        if 0 <= index < len(self.voices):
            voice_info = self.voices[index].copy()
            voice_info['full_path'] = str(self.voices_config_path.parent / voice_info['file'])
            return voice_info
        else:
            return None

    def _load_audio(self, file_path, target_sr=24000):
        """Загружает аудио и приводит к целевой частоте дискретизации."""
        try:
            audio, sr = librosa.load(file_path, sr=target_sr)
            return audio
        except Exception as e:
            print(f"Ошибка при загрузке аудио {file_path}: {e}")
            return np.zeros(self.AUDIO_LENGTH // 2)

    def _normalize_audio(self, audio, target_rms=0.15):
        """Нормализует аудио по RMS."""
        rms = np.sqrt(np.mean(audio ** 2) + 1e-8)
        if rms > 0:
            audio = audio * (target_rms / rms)
        return audio

    def _list_str_to_idx(self, text_list, vocab_map, padding_value=-1):
        """Преобразует список строк в индексы по словарю."""
        get_idx = vocab_map.get
        list_idx_tensors = [np.array([get_idx(c, 0) for c in t], dtype=np.int32) for t in text_list]
        padded_list = []
        for idx_arr in list_idx_tensors:
            if len(idx_arr) < self.TEXT_IDS_LENGTH:
                padded = np.pad(idx_arr, (0, self.TEXT_IDS_LENGTH - len(idx_arr)), constant_values=padding_value)
            else:
                padded = idx_arr[:self.TEXT_IDS_LENGTH]
            padded_list.append(padded)
        return np.array(padded_list, dtype=np.int32)

    def synthesize(self, text, output_filename="output.wav"):
        """
        Синтезирует аудио из текста с использованием ONNX моделей.
        """
        print(f"Начинается синтез для текста: {text}")
        start_time = time.time()

        selected_voice = self.get_selected_voice()
        if not selected_voice:
            print("Голос не выбран или конфигурация некорректна.")
            return False

        ref_file = selected_voice['full_path']
        ref_text = selected_voice['ref_text']

        if not os.path.exists(ref_file):
            print(f"Файл референсного аудио не найден: {ref_file}")
            return False

        # 1. Предварительная обработка текста
        accented_text = self.add_accent(text) # Использует исправленный метод
        combined_text_chars = list(ref_text + accented_text)
        combined_text = [combined_text_chars]
        text_ids_np = self._list_str_to_idx(combined_text, self.vocab_char_map)
        print(f"text_ids_np shape: {text_ids_np.shape}")

        # 2. Загрузка и предварительная обработка референсного аудио
        ref_audio = self._load_audio(ref_file, target_sr=self.SAMPLE_RATE)
        ref_audio = self._normalize_audio(ref_audio, target_rms=self.TARGET_RMS)
        ref_audio_int16 = (ref_audio * 32767).astype(np.int16)

        if len(ref_audio_int16.shape) == 1:
             ref_audio_int16 = ref_audio_int16[np.newaxis, np.newaxis, :]
        elif len(ref_audio_int16.shape) == 2 and ref_audio_int16.shape[0] == 1:
             ref_audio_int16 = ref_audio_int16[np.newaxis, :]
        else:
             ref_audio_int16 = ref_audio_int16[np.newaxis, np.newaxis, :ref_audio_int16.shape[-1]]

        audio_len = ref_audio_int16.shape[-1]
        print(f"Длина референсного аудио (сэмплов): {audio_len}")

        # 3. Вычисление max_duration
        ref_text_len = len(ref_text)
        gen_text_len = len(accented_text)

        ref_audio_len_frames = audio_len // self.HOP_LENGTH + 1
        if ref_text_len > 0:
            max_duration_val = ref_audio_len_frames + int((ref_audio_len_frames / ref_text_len) * gen_text_len / self.SPEED)
        else:
            max_duration_val = ref_audio_len_frames + int(gen_text_len / self.SPEED)

        if max_duration_val > self.MAX_SIGNAL_LENGTH:
             max_duration_val = self.MAX_SIGNAL_LENGTH
        max_duration_np = np.array([max_duration_val], dtype=np.int64)
        print(f"Вычисленное max_duration: {max_duration_val}")

        # --- Инференс ONNX ---
        try:
            # --- Модель A (Preprocess) ---
            print("Запуск ONNX Model A (Preprocess)...")
            model_a_start = time.time()
            inputs_A = {
                self.in_name_A[0]: ref_audio_int16,
                self.in_name_A[1]: text_ids_np,
                self.in_name_A[2]: max_duration_np
            }
            outputs_A = self.ort_session_A.run(None, inputs_A)
            noise, rope_cos_q, rope_sin_q, rope_cos_k, rope_sin_k, cat_mel_text, cat_mel_text_drop, ref_signal_len = outputs_A
            model_a_time = time.time() - model_a_start
            print(f"ONNX Model A завершена за {model_a_time:.2f} секунд.")
            print(f"ref_signal_len (фреймы): {ref_signal_len}") # ref_signal_len - это numpy массив из ONNX

            # --- Модель B (Transformer) ---
            print("Запуск ONNX Model B (Transformer)...")
            model_b_start = time.time()
            time_step_np = np.array([0], dtype=np.int32)
            for i in range(0, self.NFE_STEP - 1, self.FUSE_NFE):
                inputs_B = {
                    self.in_name_B[0]: noise,
                    self.in_name_B[1]: rope_cos_q,
                    self.in_name_B[2]: rope_sin_q,
                    self.in_name_B[3]: rope_cos_k,
                    self.in_name_B[4]: rope_sin_k,
                    self.in_name_B[5]: cat_mel_text,
                    self.in_name_B[6]: cat_mel_text_drop,
                    self.in_name_B[7]: time_step_np
                }
                outputs_B = self.ort_session_B.run(None, inputs_B)
                noise, time_step_np = outputs_B
                print(f"NFE_STEP: {i + self.FUSE_NFE}")
            model_b_time = time.time() - model_b_start
            print(f"ONNX Model B завершена за {model_b_time:.2f} секунд.")

            # --- Модель C (Decode) ---
            print("Запуск ONNX Model C (Decode)...")
            model_c_start = time.time()
            # --- ИСПРАВЛЕНИЕ: Убедимся, что ref_signal_len_input - 1-D массив ---
            # ref_signal_len из Model A - это numpy array. Берем первый элемент и создаем 1-D массив.
            ref_signal_len_input = np.array([ref_signal_len.item()], dtype=np.int64)
            # Альтернатива: ref_signal_len_input = ref_signal_len.flatten()[:1]

            inputs_C = {
                self.in_name_C[0]: noise,                   # denoised: float32 [1, max_duration, N_MELS]
                self.in_name_C[1]: ref_signal_len_input     # ref_signal_len: int64 [1] - ИСПРАВЛЕНИЕ
            }
            outputs_C = self.ort_session_C.run(None, inputs_C)
            generated_signal_int16 = outputs_C[0] # int16 [1, 1, generated_len]
            model_c_time = time.time() - model_c_start
            print(f"ONNX Model C завершена за {model_c_time:.2f} секунд.")
            # --- Конец исправления ---

            # --- Сохранение аудио ---
            total_time = time.time() - start_time
            print(f"Синтез завершен за {total_time:.2f} секунд.")
            print(f"  - Model A: {model_a_time:.2f}s")
            print(f"  - Model B: {model_b_time:.2f}s")
            print(f"  - Model C: {model_c_time:.2f}s")

            audio_data = generated_signal_int16.squeeze()
            if audio_data.dtype != np.int16:
                 print(f"Предупреждение: тип данных аудио {audio_data.dtype}, преобразую в int16.")
                 audio_data = audio_data.astype(np.int16)

            output_path = Path(output_filename).resolve()
            os.makedirs(output_path.parent, exist_ok=True)
            sf.write(str(output_path), audio_data, self.SAMPLE_RATE, format='WAV')
            print(f"Аудио сохранено в {output_path}")
            return True

        except Exception as e:
            print(f"Ошибка во время инференса ONNX: {e}")
            import traceback
            traceback.print_exc()
            return False
