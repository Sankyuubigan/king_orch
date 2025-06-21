# agents/base_agent.py - ТЕПЕРЬ ПРИНИМАЕТ ГОТОВЫЙ ОБЪЕКТ МОДЕЛИ

from llama_cpp import Llama
import json
import requests

class BaseAgent:
    def __init__(self, llm_instance: Llama, system_prompt: str, tools_config: dict, log_callback):
        self.log = log_callback
        self.system_prompt = system_prompt
        self.tools_config = tools_config
        self.llm = llm_instance # Используем переданный экземпляр, а не загружаем новый
        self.history = []

    def _create_prompt(self):
        system_message = {"role": "system", "content": self.system_prompt}
        full_history = [system_message] + self.history
        prompt_str = ""
        for message in full_history:
            prompt_str += f"<|im_start|>{message['role']}\n{message['content']}<|im_end|>\n"
        prompt_str += "<|im_start|>assistant\n"
        return prompt_str

    def _execute_tool(self, tool_name: str, tool_params: dict) -> str:
        if tool_name not in self.tools_config:
            return f"Ошибка: неизвестный инструмент '{tool_name}'"
        
        tool_info = self.tools_config[tool_name]
        url = tool_info["url"]

        try:
            if "read" in tool_name or "list" in tool_name:
                response = requests.get(url, params=tool_params, timeout=20)
            else:
                response = requests.post(url, json=tool_params, timeout=20)
            
            response.raise_for_status()
            return response.json().get("result", "Инструмент не вернул результат.")
        except Exception as e:
            self.log(f"[{self.__class__.__name__}] Ошибка вызова инструмента: {e}")
            return f"Ошибка: {e}"

    def execute_step(self, current_task: str) -> (str, bool):
        self.history.append({"role": "user", "content": current_task})
        prompt = self._create_prompt()
        
        self.log(f"[{self.__class__.__name__}] Думаю над задачей: {current_task[:80]}...")
        output = self.llm(prompt, max_tokens=1024, stop=["<|im_end|>"])
        response_text = output['choices'][0]['text'].strip()
        
        self.history.append({"role": "assistant", "content": response_text})

        if "[TOOL_CALL]" in response_text:
            try:
                tool_call_str = response_text.split("[TOOL_CALL]")[1].strip()
                tool_call = json.loads(tool_call_str)
                tool_name = tool_call.get("tool")
                tool_params = tool_call.get("params", {})
                
                self.log(f"[{self.__class__.__name__}] Решил использовать '{tool_name}' с параметрами {tool_params}")
                tool_result = self._execute_tool(tool_name, tool_params)
                self.history.append({"role": "tool", "content": tool_result})
                return tool_result, False
            except Exception as e:
                error_msg = f"Ошибка парсинга или вызова инструмента: {e}"
                self.history.append({"role": "tool", "content": error_msg})
                return error_msg, False
        else:
            return response_text, True