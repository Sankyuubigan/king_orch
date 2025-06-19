# tools.py
from langchain_core.tools import BaseTool

class StagehandTool(BaseTool):
    name: str = "Stagehand Инструмент"
    description: str = "Принимает текстовый запрос для Stagehand и выполняет его."
    def _run(self, query: str) -> str:
        return f"Инструмент Stagehand успешно выполнен с запросом: '{query}'"

class CalculatorTool(BaseTool):
    name: str = "Калькулятор"
    description: str = "Вычисляет математические выражения."
    def _run(self, expression: str) -> str:
        try:
            return str(eval(expression))
        except Exception as e:
            return f"Ошибка вычисления: {e}"

ALL_TOOLS = { "Stagehand": StagehandTool(), "Калькулятор": CalculatorTool() }