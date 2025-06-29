from queue import Queue

class UiAdapter:
    """
    Адаптер, который преобразует вызовы методов от CoreWorker
    в потокобезопасные сообщения для UI.
    Он реализует интерфейс "слушателя" для CoreWorker.
    """
    def __init__(self, ui_update_queue: Queue):
        self.queue = ui_update_queue

    def on_status_changed(self, status_text: str):
        """Вызывается, когда CoreWorker меняет свой статус."""
        self.queue.put(('set_info_label', (status_text,)))

    def on_busy_changed(self, is_busy: bool, status_text: str = None):
        """Вызывается, когда CoreWorker начинает или заканчивает долгую операцию."""
        self.queue.put(('set_ui_busy', (is_busy, status_text)))

    def on_final_result(self, result_text: str):
        """Вызывается, когда CoreWorker сгенерировал финальный ответ."""
        self.queue.put(('update_chat_with_final_result', (result_text,)))

    def on_log_message(self, message: str):
        """Вызывается, когда CoreWorker хочет что-то записать в лог UI."""
        self.queue.put(('log_to_widget', (message,)))
        
    def on_critical_error(self, error_text: str):
        """Вызывается при неустранимой ошибке в CoreWorker."""
        self.queue.put(('set_info_label', (error_text,)))
        self.queue.put(('unlock_settings_button', ()))