# utils/prompt_templates.py - Финальная, исправленная версия для создания обработчика чата

from llama_cpp.llama_chat_format import Jinja2ChatFormatter
from datetime import datetime

def get_fixed_chat_handler():
    """
    Создает и возвращает исправленный обработчик чата LlamaCPP.
    Эта функция принудительно внедряет недостающую функцию 'strftime_now'
    в окружение Jinja2, что гарантированно решает ошибку 'UndefinedError'.
    """
    
    # Шаблон, который использует проблемная модель.
    template = (
        "{% for message in messages %}"
        "{{'<|im_start|>' + message['role'] + '\n' + message['content'] + '<|im_end|>' + '\n'}}"
        "{% endfor %}"
        "{% if add_generation_prompt %}"
        "{{ '<|im_start|>assistant\n' }}"
        "{% endif %}"
    )

    formatter = Jinja2ChatFormatter(
        template=template,
        eos_token="<|im_end|>",
        bos_token=""
    )

    def strftime_now(format_str: str) -> str:
        """Недостающая функция, которую требует шаблон."""
        return datetime.now().strftime(format_str)

    # ГЛАВНОЕ ИСПРАВЛЕНИЕ: Используем правильное имя внутреннего атрибута `_environment`
    # для доступа к глобальным переменным шаблонизатора.
    formatter._environment.globals["strftime_now"] = strftime_now

    return formatter.to_chat_handler()