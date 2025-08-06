import sys
from pathlib import Path

def main():
    project_root = Path(__file__).parent.parent
    onnx_repo_path = project_root / "F5-TTS-ONNX"

    if not onnx_repo_path.exists():
        print("Сначала скачайте F5-TTS-ONNX:")
        print("git clone https://github.com/DakeQQ/F5-TTS-ONNX.git")
        return

    # --- Исправление путей ---
    # Путь к папке, где лежит STFT_Process.py и Export_F5.py
    f5_tts_module_path = onnx_repo_path / "Export_ONNX" / "F5_TTS"
    # Добавляем этот путь в sys.path *первым*
    sys.path.insert(0, str(f5_tts_module_path))
    # Также может потребоваться добавить корень репо, если там есть другие зависимости
    sys.path.insert(1, str(onnx_repo_path))

    # --- Остальной код ---
    # Проверка существования файлов
    export_script = f5_tts_module_path / "Export_F5.py"
    stft_module = f5_tts_module_path / "STFT_Process.py"

    if not export_script.exists():
        print(f"[ERROR] Скрипт экспорта не найден: {export_script}")
        return
    if not stft_module.exists():
        print(f"[ERROR] Модуль STFT_Process.py не найден: {stft_module}")
        return

    try:
        # Теперь этот импорт должен сработать
        from Export_F5 import main as export_onnx # Импорт без префикса, так как путь добавлен
        print("[INFO] Модуль экспорта успешно импортирован.")
    except Exception as e:
        print(f"[ERROR] Ошибка импорта модуля экспорта: {e}")
        import traceback
        traceback.print_exc()
        return

    # Путь к модели
    model_path = Path(r"D:\nn\models\tts\Misha2410-F5-TTS_RUSSIAN")
    output_dir = model_path / "onnx"
    output_dir.mkdir(parents=True, exist_ok=True)
    output_path = output_dir / "model.onnx"

    # Запуск экспорта
    print("Начинаем конвертацию в ONNX...")
    try:
        # Убедись, что сигнатура функции main в Export_F5.py соответствует этим аргументам
        export_onnx(
            model_path=str(model_path),
            output_path=str(output_path),
            use_fp16_transformer=True,
            
        )
        print(f"ONNX модель успешно сохранена в {output_path}")
    except Exception as e:
        print(f"Ошибка при конвертации в ONNX: {e}")
        import traceback
        traceback.print_exc()

if __name__ == "__main__":
    main()