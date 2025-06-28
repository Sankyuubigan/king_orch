import logging
import os
import sys

def setup_logging():
    """
    Настраивает централизованное логирование для вывода в консоль и файл.
    """
    LOGS_DIR = "logs"
    LOG_FILE = os.path.join(LOGS_DIR, "app_trace.log")

    # Создаем директорию для логов, если она не существует
    os.makedirs(LOGS_DIR, exist_ok=True)

    # Получаем корневой логгер
    logger = logging.getLogger()
    
    # Устанавливаем уровень логирования. INFO - для общих шагов, DEBUG - для деталей.
    logger.setLevel(logging.INFO)

    # Предотвращаем дублирование обработчиков, если функция вызывается повторно
    if logger.hasHandlers():
        logger.handlers.clear()

    # Форматтер для логов
    formatter = logging.Formatter(
        '%(asctime)s - %(name)s - %(levelname)s - %(message)s',
        datefmt='%Y-%m-%d %H:%M:%S'
    )

    # Обработчик для вывода в консоль
    stream_handler = logging.StreamHandler(sys.stdout)
    stream_handler.setFormatter(formatter)
    logger.addHandler(stream_handler)

    # Обработчик для записи в файл
    file_handler = logging.FileHandler(LOG_FILE, mode='a', encoding='utf-8')
    file_handler.setFormatter(formatter)
    logger.addHandler(file_handler)

    logging.info("Система логирования успешно настроена.")