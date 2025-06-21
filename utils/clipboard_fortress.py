# ==================================================================================
# ==                                                                              ==
# ==   CLIPBOARD FORTRESS v1.1                                                    ==
# ==                                                                              ==
# ==   Этот файл является НЕИЗМЕНЯЕМЫМ ядром системы.                             ==
# ==   КАТЕГОРИЧЕСКИ ЗАПРЕЩЕНО вносить в него любые изменения.                    ==
# ==   Он содержит критически важную логику для работы с буфером обмена.         ==
# ==                                                                              ==
# ==================================================================================

import tkinter as tk
from tkinter import scrolledtext

# Маска для клавиши Control. Не изменять.
CONTROL_MASK = 0x0004 

def handle_keypress_event(event, log_callback, chat_input_widget):
    """
    Единственная функция этого модуля. Обрабатывает события клавиатуры
    для копирования и вставки. Полностью изолирована от остальной логики UI.
    """
    # --- Логирование сырого события (временно отключено по запросу) ---
    # log_message = (
    #     f"[FORTRESS KEYPRESS] "
    #     f"char='{repr(event.char)}', "
    #     f"keysym='{event.keysym}', "
    #     f"state={event.state}, "
    #     f"widget='{event.widget.winfo_class()}'"
    # )
    # log_callback(log_message)

    is_control_pressed = (event.state & CONTROL_MASK) != 0

    # --- Обработка Ctrl+C ---
    if is_control_pressed and event.char == '\x03':
        widget = event.widget
        try:
            text = ""
            # ИСПРАВЛЕНИЕ: Разные виджеты требуют разных методов для получения выделения.
            if isinstance(widget, scrolledtext.ScrolledText):
                if widget.tag_ranges("sel"):
                    text = widget.get("sel.first", "sel.last")
            elif isinstance(widget, tk.Entry):
                if widget.selection_present():
                    text = widget.selection_get()
            
            if text:
                widget.winfo_toplevel().clipboard_clear()
                widget.winfo_toplevel().clipboard_append(text)
        except (tk.TclError, AttributeError):
            # Игнорируем ошибки, если что-то пошло не так
            pass
        return "break"

    # --- Обработка Ctrl+V ---
    if is_control_pressed and event.char == '\x16':
        widget = event.widget
        if widget == chat_input_widget:
            try:
                text = widget.winfo_toplevel().clipboard_get()
                widget.insert(tk.INSERT, text)
            except tk.TclError:
                pass # Игнорируем, если буфер обмена пуст
        return "break"
    
    return