# inspect_library.py

import pkgutil
import importlib
import inspect
import sys

try:
    # Пытаемся импортировать базовый пакет, чтобы получить его путь
    import copilotkit
except ImportError:
    print("FATAL: The 'copilotkit' package is not installed correctly.")
    sys.exit(1)

def inspect_package(package):
    """
    Проходит по пакету и выводит на экран его модули и классы.
    """
    print(f"--- Начинаю инспекцию пакета: {package.__name__} ---")
    print(f"--- Расположение: {package.__path__} ---\n")
    
    module_count = 0
    class_count = 0

    # pkgutil.walk_packages находит все модули внутри пакета
    for _, modname, _ in pkgutil.walk_packages(path=package.__path__,
                                               prefix=package.__name__ + '.',
                                               onerror=lambda x: None):
        try:
            # Динамически импортируем найденный модуль
            module = importlib.import_module(modname)
            module_count += 1
            
            # Ищем классы, которые определены именно в этом модуле
            classes = [name for name, obj in inspect.getmembers(module, inspect.isclass) 
                       if obj.__module__ == modname]
            
            if classes:
                print(f"[+] Найден модуль: {modname}")
                for cls_name in classes:
                    print(f"    - КЛАСС: {cls_name}")
                    class_count += 1

        except Exception as e:
            # Игнорируем модули, которые не удалось импортировать
            pass

    print(f"\n--- Инспекция завершена ---")
    print(f"Найдено модулей: {module_count}")
    print(f"Найдено классов: {class_count}")
    if class_count == 0:
        print("\n[!!!] ВНИМАНИЕ: Не найдено ни одного кастомного класса. Возможно, пакет пуст или установлен некорректно.")


if __name__ == "__main__":
    inspect_package(copilotkit)