# app_flet.py
import flet as ft
import os
import sys
from pathlib import Path

# --- Настройки ---
# Укажите правильный путь к папке с моделью
MODEL_PATH = r"D:\nn\models\tts\Misha2410-F5-TTS_RUSSIAN" # <--- ИЗМЕНИТЕ ЭТО
VOICES_CONFIG_PATH = "voice_engine/voices/voices.json"
SETTINGS_PATH = "voice_engine/settings.json"
# --- Конец настроек ---

# Добавляем путь к voice_engine для импорта
sys.path.append(str(Path(__file__).parent))

from voice_engine.tts_core import TTSEngine

def main(page: ft.Page):
    page.title = "F5-TTS (Русский)"
    page.window_width = 600
    page.window_height = 500
    page.window_min_width = 400
    page.window_min_height = 400
    page.theme_mode = ft.ThemeMode.SYSTEM # Или LIGHT / DARK
    page.padding = 10
    page.spacing = 10

    # --- Инициализация TTS ---
    tts_engine = TTSEngine(MODEL_PATH, VOICES_CONFIG_PATH, SETTINGS_PATH)

    # --- UI Elements ---
    # Статусная строка
    status_text = ft.Text("", size=12, color=ft.colors.GREY)

    # Поле ввода текста
    text_field = ft.TextField(
        label="Введите текст для озвучивания",
        multiline=True,
        min_lines=3,
        max_lines=5,
        expand=True,
        text_size=14
    )

    # Кнопка "Произнести"
    def speak_clicked(e):
        text = text_field.value.strip()
        if not text:
            status_text.value = "Ошибка: Введите текст."
            status_text.color = ft.colors.RED
            page.update()
            return

        status_text.value = "Генерация и воспроизведение..."
        status_text.color = ft.colors.BLUE
        page.update() # Обновляем статус немедленно

        # Запускаем синтез и воспроизведение
        success = tts_engine.synthesize_and_play(text)
        if success:
            status_text.value = "Воспроизведение..."
            status_text.color = ft.colors.GREEN
        else:
            status_text.value = "Ошибка при генерации или воспроизведении."
            status_text.color = ft.colors.RED
        page.update()

    speak_button = ft.ElevatedButton("Произнести", on_click=speak_clicked, expand=True)

    # Кнопка "Стоп"
    def stop_clicked(e):
        tts_engine.stop_playback_now()
        status_text.value = "Воспроизведение остановлено."
        status_text.color = ft.colors.ORANGE
        page.update()

    stop_button = ft.ElevatedButton("Стоп", on_click=stop_clicked, expand=True, bgcolor=ft.colors.RED_50, color=ft.colors.WHITE)

    # --- Настройки (модальное окно) ---
    # Выпадающий список голосов
    voice_dropdown = ft.Dropdown(
        label="Выберите голос",
        options=[],
        width=300
    )

    # Заполняем список голосов
    def update_voice_dropdown():
        voice_names = tts_engine.get_voice_list()
        voice_dropdown.options = [ft.dropdown.Option(name) for name in voice_names]
        if voice_names:
            # Устанавливаем выбранный голос из настроек
            current_settings = tts_engine.current_settings
            current_index = current_settings.get('selected_voice_index', 0)
            if 0 <= current_index < len(voice_names):
                voice_dropdown.value = voice_names[current_index]
        page.update()

    update_voice_dropdown() # Инициализируем при запуске

    # Обработчик изменения голоса
    def voice_changed(e):
        selected_name = voice_dropdown.value
        voice_names = tts_engine.get_voice_list()
        if selected_name in voice_names:
            index = voice_names.index(selected_name)
            tts_engine.set_selected_voice(index)
            status_text.value = f"Голос изменен на: {selected_name}"
            status_text.color = ft.colors.GREEN
            page.update()

    voice_dropdown.on_change = voice_changed

    # Кнопка закрытия настроек
    def close_settings(e):
        settings_modal.open = False
        page.update()

    close_settings_btn = ft.TextButton("Закрыть", on_click=close_settings)

    # Модальное окно настроек
    settings_content = ft.Column(
        [
            ft.Text("Настройки", size=20, weight=ft.FontWeight.BOLD),
            voice_dropdown,
            ft.Divider(),
            ft.Text("Информация о выбранном голосе:", size=14, weight=ft.FontWeight.W_500),
            # Информация будет обновляться динамически
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

    # Функция для обновления информации о голосе в настройках
    def update_voice_info_in_modal():
        # Очищаем предыдущую информацию
        # Находим индекс Text виджета с информацией (последний элемент перед кнопкой закрытия)
        info_widgets_start_index = None
        for i, widget in enumerate(settings_content.controls):
            if isinstance(widget, ft.Text) and widget.value == "Информация о выбранном голосе:":
                info_widgets_start_index = i + 1
                break

        if info_widgets_start_index is not None:
            # Удаляем старые виджеты информации
            while len(settings_content.controls) > info_widgets_start_index + 1: # +1 for Divider
                 settings_content.controls.pop(info_widgets_start_index + 1) # +1 for Divider

            # Добавляем новую информацию
            selected_voice = tts_engine.get_selected_voice()
            if selected_voice:
                settings_content.controls.append(ft.Text(f"Имя: {selected_voice['name']}", size=12))
                settings_content.controls.append(ft.Text(f"Файл: {selected_voice['file']}", size=12))
                settings_content.controls.append(ft.Text("Текст образца:", size=12, weight=ft.FontWeight.W_500))
                settings_content.controls.append(ft.TextField(value=selected_voice['ref_text'], read_only=True, multiline=True, text_size=11))

        page.update()

    # Обновляем информацию при открытии модального окна
    def open_settings(e):
        update_voice_dropdown() # На случай, если список изменился
        update_voice_info_in_modal()
        page.dialog = settings_modal
        settings_modal.open = True
        page.update()

    # Кнопка настроек (шестерёнка)
    settings_button = ft.IconButton(icon=ft.icons.SETTINGS, on_click=open_settings, tooltip="Настройки")

    # --- Layout ---
    page.add(
        ft.Row([ft.Text("F5-TTS Синтезатор Речи", size=24, weight=ft.FontWeight.BOLD)], alignment=ft.MainAxisAlignment.CENTER),
        ft.Divider(height=9, thickness=2),
        text_field,
        ft.Row([speak_button, stop_button], alignment=ft.MainAxisAlignment.SPACE_BETWEEN),
        ft.Divider(height=9, thickness=1),
        ft.Row([status_text], alignment=ft.MainAxisAlignment.START),
        ft.Row([settings_button], alignment=ft.MainAxisAlignment.END)
    )

# Запуск приложения Flet
ft.app(target=main)
# Или для веб-сервера: ft.app(target=main, view=ft.WEB_BROWSER)