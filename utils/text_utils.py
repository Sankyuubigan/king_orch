import re
from typing import Dict

def extract_thoughts(text: str) -> Dict[str, str]:
    """
    Извлекает "мысли" (<think>...</think>) из текста и отделяет их от основного ответа.
    Простая и надежная версия, рассчитанная на формат "мысли -> ответ".
    """
    thoughts = None
    answer = text # По умолчанию, весь текст - это ответ

    # re.DOTALL позволяет точке (.) соответствовать символу новой строки.
    match = re.search(r"<think>(.*?)</think>", text, re.DOTALL)
    
    if match:
        thoughts = match.group(1).strip()
        # Ответом является все, что идет ПОСЛЕ закрывающего тега </think>
        answer = text[match.end():].strip()
    
    return {"answer": answer, "thoughts": thoughts}