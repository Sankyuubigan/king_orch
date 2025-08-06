# tests/app_flet_onnx.py
import flet as ft
# import flet.colors as colors # Не используется
import os
import sys
import threading
from pathlib import Path

# --- Настройки путей относительно корня проекта ---
PROJECT_ROOT = Path(__file__).parent.parent.resolve()
print(f"[DEBUG] Корень проекта: {PROJECT_ROOT}")

MODEL_DIR = Path(r"D:\nn\models\tts\Misha2410-F5-TTS_RUSSIAN") # Твоя строка
ONNX_MODEL_DIR = MODEL_DIR / "onnx"
VOICE_ENGINE_DIR = PROJECT_ROOT / "voice_engine"

ONNX_MODEL_A_PATH = ONNX_MODEL_DIR / "F5_Preprocess.onnx"
ONNX_MODEL_B_PATH = ONNX_MODEL_DIR / "F5_Transformer.onnx"
ONNX_MODEL_C_PATH = ONNX_MODEL_DIR / "F5_Decode.onnx"

VOICES_CONFIG_PATH = VOICE_ENGINE_DIR / "voices" / "voices.json"
SETTINGS_PATH = VOICE_ENGINE_DIR / "settings.json"
VOCAB_PATH = MODEL_DIR / "vocab.txt"
# --- Конец настроек путей ---

# Импортируем движок TTS
sys.path.append(str(PROJECT_ROOT / "tests")) # Добавляем tests в путь для импорта
from onnx_tts_engine import ONNXTTSEngine

def main(page: ft.Page):
    page.title = "F5-TTS (ONNX)"
    page.window_width = 600
    page.window_height = 500
    page.window_min_width = 400
    page.window_min_height = 400
    page.theme_mode = ft.ThemeMode.SYSTEM
    page.padding = 10
    page.spacing = 10

    # --- Инициализация TTS Engine ---
    try:
        tts_engine = ONNXTTSEngine(
            model_a_path=ONNX_MODEL_A_PATH,
            model_b_path=ONNX_MODEL_B_PATH,
            model_c_path=ONNX_MODEL_C_PATH,
            vocab_path=VOCAB_PATH,
            voices_config_path=VOICES_CONFIG_PATH,
            settings_path=SETTINGS_PATH
        )
        engine_status = "ONNX модели загружены"
        # ИСПРАВЛЕНИЕ: Используем цвет как строку
        engine_status_color = "green"
    except Exception as e:
        print(f"Критическая ошибка при инициализации ONNX TTS Engine: {e}")
        import traceback
        traceback.print_exc()
        tts_engine = None
        engine_status = f"Ошибка загрузки ONNX"
        # ИСПРАВЛЕНИЕ: Используем цвет как строку
        engine_status_color = "red"

    # --- UI Elements ---
    # ИСПРАВЛЕНИЕ: Используем цвет как строку
    status_text = ft.Text(engine_status, size=12, color=engine_status_color)

    text_field = ft.TextField(
        label="Введите текст для озвучивания",
        multiline=True,
        min_lines=3,
        max_lines=5,
        expand=True,
        text_size=14,
        value="Привет, это тест синтеза речи через ONNX."
    )

    def speak_clicked(e):
        if not tts_engine:
            status_text.value = "Ошибка: Движок TTS не инициализирован."
            # ИСПРАВЛЕНИЕ: Используем цвет как строку
            status_text.color = "red"
            page.update()
            return

        text = text_field.value.strip()
        if not text:
            status_text.value = "Ошибка: Введите текст."
            # ИСПРАВЛЕНИЕ: Используем цвет как строку
            status_text.color = "red"
            page.update()
            return

        status_text.value = "Генерация аудио через ONNX..."
        # ИСПРАВЛЕНИЕ: Используем цвет как строку
        status_text.color = "blue"
        page.update()

        def run_synthesis():
            output_file = str(PROJECT_ROOT / "tests" / "generated_onnx.wav")
            success = tts_engine.synthesize(text, output_filename=output_file)
            if success:
                status_text.value = f"Аудио сгенерировано и сохранено в {output_file}"
                # ИСПРАВЛЕНИЕ: Используем цвет как строку
                status_text.color = "green"
                # try:
                #     os.startfile(output_file) # Только на Windows
                # except Exception as ex:
                #     print(f"Не удалось открыть файл: {ex}")
            else:
                status_text.value = "Ошибка при генерации аудио."
                # ИСПРАВЛЕНИЕ: Используем цвет как строку
                status_text.color = "red"
            page.update()

        threading.Thread(target=run_synthesis, daemon=True).start()

    speak_button = ft.ElevatedButton("Произнести (ONNX)", on_click=speak_clicked, expand=True)

    # --- Настройки (модальное окно) ---
    voice_dropdown = ft.Dropdown(
        label="Выберите голос",
        options=[],
        width=300
    )

    def update_voice_dropdown():
        if not tts_engine:
            return
        voice_names = tts_engine.get_voice_list()
        voice_dropdown.options = [ft.dropdown.Option(name) for name in voice_names]
        if voice_names:
            current_settings = tts_engine.current_settings
            current_index = current_settings.get('selected_voice_index', 0)
            if 0 <= current_index < len(voice_names):
                voice_dropdown.value = voice_names[current_index]
        page.update()

    update_voice_dropdown()

    def voice_changed(e):
        if not tts_engine:
            return
        selected_name = voice_dropdown.value
        voice_names = tts_engine.get_voice_list()
        if selected_name in voice_names:
            index = voice_names.index(selected_name)
            tts_engine.set_selected_voice(index)
            status_text.value = f"Голос изменен на: {selected_name}"
            # ИСПРАВЛЕНИЕ: Используем цвет как строку
            status_text.color = "green"
            page.update()

    voice_dropdown.on_change = voice_changed

    def close_settings(e):
        settings_modal.open = False
        page.update()

    close_settings_btn = ft.TextButton("Закрыть", on_click=close_settings)

    settings_content = ft.Column(
        [
            ft.Text("Настройки", size=20, weight=ft.FontWeight.BOLD),
            voice_dropdown,
            ft.Divider(),
            ft.Text("Информация о выбранном голосе:", size=14, weight=ft.FontWeight.W_500),
        ],
        spacing=10,
        scroll=ft.ScrollMode.AUTO
    )
    settings_modal = ft.AlertDialog(
        modal=True,
        content=settings_content,
        actions=[close_settings_btn],
        actions_alignment=ft.MainAxisAlignment.END,
    )

    def update_voice_info_in_modal():
        if not tts_engine:
            return
        info_widgets_start_index = None
        for i, widget in enumerate(settings_content.controls):
            if isinstance(widget, ft.Text) and widget.value == "Информация о выбранном голосе:":
                info_widgets_start_index = i + 1
                break

        if info_widgets_start_index is not None:
            while len(settings_content.controls) > info_widgets_start_index + 1:
                 settings_content.controls.pop(info_widgets_start_index + 1)

            selected_voice = tts_engine.get_selected_voice()
            if selected_voice:
                settings_content.controls.append(ft.Text(f"Имя: {selected_voice['name']}", size=12))
                settings_content.controls.append(ft.Text(f"Файл: {selected_voice['file']}", size=12))
                settings_content.controls.append(ft.Text("Текст образца:", size=12, weight=ft.FontWeight.W_500))
                settings_content.controls.append(ft.TextField(value=selected_voice['ref_text'], read_only=True, multiline=True, text_size=11))

        page.update()

    def open_settings(e):
        update_voice_dropdown()
        update_voice_info_in_modal()
        page.dialog = settings_modal
        settings_modal.open = True
        page.update()

    # --- ИСПРАВЛЕНИЕ: Используем строку для иконки ---
    settings_button = ft.IconButton(icon="settings", on_click=open_settings, tooltip="Настройки")
    # --- Конец исправления ---

    # --- Layout ---
    page.add(
        ft.Row([ft.Text("F5-TTS Синтезатор Речи (ONNX)", size=24, weight=ft.FontWeight.BOLD)], alignment=ft.MainAxisAlignment.CENTER),
        ft.Divider(height=9, thickness=2),
        text_field,
        ft.Row([speak_button], alignment=ft.MainAxisAlignment.CENTER),
        ft.Divider(height=9, thickness=1),
        ft.Row([status_text], alignment=ft.MainAxisAlignment.START),
        ft.Row([settings_button], alignment=ft.MainAxisAlignment.END)
    )

# --- Запуск приложения Flet ---
if __name__ == "__main__":
    required_packages = ['flet', 'onnxruntime', 'soundfile', 'librosa', 'numpy']
    missing_packages = []
    for pkg in required_packages:
        try:
            __import__(pkg)
        except ImportError:
            missing_packages.append(pkg)

    if missing_packages:
        error_msg = f"Пожалуйста, установите недостающие пакеты: {', '.join(missing_packages)}"
        print(error_msg)
        print("Пример: pip install flet onnxruntime soundfile librosa numpy")
        if 'ruaccent' not in missing_packages:
             print("Также рекомендуется установить ruaccent: pip install ruaccent")
        # ИСПРАВЛЕНИЕ: Используем цвет как строку
        def show_error(page: ft.Page):
             # ИСПРАВЛЕНИЕ: Используем цвет как строку
             page.add(ft.Text(error_msg, color="red"))
        ft.app(target=show_error)
        exit(1)

    ft.app(target=main)
    # Или для веб-сервера: ft.app(target=main, view=ft.WEB_BROWSER)
