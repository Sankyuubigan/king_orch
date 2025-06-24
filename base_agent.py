# base_agent.py - НАУЧИЛСЯ РАБОТАТЬ С ВНУТРЕННИМИ ИНСТРУМЕНТАМИ

from llama_cpp import Llama
import json
import requests
import re

class BaseAgent:
    def __init__(self, llm_instance: Llama, system_prompt: str, tools_config: dict, log_callback, agent_router=None, internal_router=None):
        self.log = log_callback
        self.system_prompt = system_prompt
        self.tools_config = tools_config
        self.llm = llm_instance
        self.history = []
        self.agent_router = agent_router
        self.internal_router = internal_router # <-- Роутер для внутренних инструментов

    def _execute_tool(self, tool_name: str, tool_params: dict) -> str:
        if tool_name not in self.tools_config:
            return f"Ошибка: неизвестный инструмент '{tool_name}'"
        
        tool_info = self.tools_config[tool_name]
        
        # ИЗМЕНЕНО: Проверяем тип инструмента
        if tool_info.get("type") == "internal":
            if not self.internal_router:
                return f"Ошибка: агент не уполномочен вызывать внутренние инструменты."
            self.log(f"[{self.__class__.__name__}] Вызываю внутренний инструмент '{tool_name}'...")
            return self.internal_router(tool_name, tool_params)

        url = tool_info.get("url")
        if not url:
            return f"Ошибка: URL для инструмента '{tool_name}' не определен."
            
        try:
            if tool_name in ["file_reader", "file_lister"]:
                response = requests.get(url, params=tool_params, timeout=30)
            else:
                response = requests.post(url, json=tool_params, timeout=30)
            
            response.raise_for_status()
            try: return response.json()
            except json.JSONDecodeError: return response.text
                
        except Exception as e:
            self.log(f"[{self.__class__.__name__}] Ошибка вызова инструмента '{tool_name}': {e}")
            return f"Ошибка: {e}"

    # ... остальной код без изменений ...
    def _create_prompt(self):
        system_message = {"role": "system", "content": self.system_prompt}
        full_history = [system_message] + self.history
        prompt_str = ""
        for message in full_history:
            prompt_str += f"<|im_start|>{message['role']}\n{message['content']}<|im_end|>\n"
        prompt_str += "<|im_start|>assistant\n"
        return prompt_str
    def execute_step(self, current_task: str) -> (str, bool):
        self.history.append({"role": "user", "content": current_task})
        prompt = self._create_prompt()
        self.log(f"[{self.__class__.__name__}] Думаю над задачей: {current_task[:80]}...")
        output = self.llm(prompt, max_tokens=1024, stop=["<|im_end|>"])
        response_text = output['choices'][0]['text'].strip()
        self.history.append({"role": "assistant", "content": response_text})
        if "[AGENT_CALL]" in response_text:
            if not self.agent_router:
                error_msg = "Ошибка: этот агент не уполномочен вызывать других агентов."
                self.history.append({"role": "agent", "content": error_msg})
                return error_msg, False
            try:
                match = re.search(r'\{[\s\S]*\}', response_text)
                if not match: raise ValueError("JSON-объект для вызова агента не найден.")
                agent_call = json.loads(match.group(0))
                agent_name, agent_task = agent_call.get("agent"), agent_call.get("task", "")
                self.log(f"[{self.__class__.__name__}] Делегирую задачу '{agent_task[:50]}...' агенту '{agent_name}'")
                agent_result = self.agent_router(agent_name, agent_task)
                result_str = json.dumps(agent_result, ensure_ascii=False, indent=2) if isinstance(agent_result, (dict, list)) else str(agent_result)
                self.history.append({"role": "agent", "content": result_str})
                return agent_result, False
            except Exception as e:
                error_msg = f"Ошибка вызова агента: {e}"
                self.history.append({"role": "agent", "content": error_msg})
                return error_msg, False
        elif "[TOOL_CALL]" in response_text:
            try:
                match = re.search(r'\{[\s\S]*\}', response_text)
                if not match: raise ValueError("JSON-объект для вызова инструмента не найден.")
                tool_call = json.loads(match.group(0))
                tool_name, tool_params = tool_call.get("tool"), tool_call.get("params", {})
                self.log(f"[{self.__class__.__name__}] Использую '{tool_name}' с параметрами {tool_params}")
                tool_result = self._execute_tool(tool_name, tool_params)
                tool_result_str = json.dumps(tool_result, ensure_ascii=False, indent=2) if isinstance(tool_result, (dict, list)) else str(tool_result)
                self.history.append({"role": "tool", "content": tool_result_str})
                return tool_result, False 
            except Exception as e:
                error_msg = f"Ошибка парсинга или вызова инструмента: {e}"
                self.history.append({"role": "tool", "content": error_msg})
                return error_msg, False
        else:
            return response_text, True
    def work_on_task(self, task: str) -> str:
        self.history = []
        final_result, _ = self.execute_step(task)
        return final_result