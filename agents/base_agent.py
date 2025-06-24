# agents/base_agent.py - УЛУЧШЕН ПАРСИНГ JSON ДЛЯ НАДЕЖНОСТИ

from llama_cpp import Llama
import json
import requests
import re # <-- Импортируем модуль для регулярных выражений

class BaseAgent:
    def __init__(self, llm_instance: Llama, system_prompt: str, tools_config: dict, log_callback, agent_router=None):
        self.log = log_callback
        self.system_prompt = system_prompt
        self.tools_config = tools_config
        self.llm = llm_instance
        self.history = []
        self.agent_router = agent_router # Маршрутизатор для вызова других агентов

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
            # Простая и надежная логика: GET для чтения, POST для всего остального
            if tool_name in ["file_reader", "file_lister"]:
                response = requests.get(url, params=tool_params, timeout=30)
            else:
                response = requests.post(url, json=tool_params, timeout=30)
            
            response.raise_for_status()
            try:
                # Пытаемся вернуть JSON, если не получается - возвращаем текст
                return response.json()
            except json.JSONDecodeError:
                return response.text
                
        except Exception as e:
            self.log(f"[{self.__class__.__name__}] Ошибка вызова инструмента '{tool_name}': {e}")
            return f"Ошибка: {e}"

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
                # ИЗМЕНЕНО: Используем регулярное выражение для надежного извлечения JSON
                match = re.search(r'\{[\s\S]*\}', response_text)
                if not match:
                    raise ValueError("JSON-объект для вызова агента не найден в ответе модели.")
                
                agent_call_str = match.group(0)
                agent_call = json.loads(agent_call_str)
                agent_name = agent_call.get("agent")
                agent_task = agent_call.get("task", "")
                
                self.log(f"[{self.__class__.__name__}] Решил делегировать задачу '{agent_task[:50]}...' агенту '{agent_name}'")
                agent_result = self.agent_router(agent_name, agent_task)
                
                result_str = json.dumps(agent_result, ensure_ascii=False, indent=2) if isinstance(agent_result, (dict, list)) else str(agent_result)
                self.history.append({"role": "agent", "content": result_str}) # Новый тип роли
                return agent_result, False
            except Exception as e:
                error_msg = f"Ошибка вызова агента: {e}"
                self.history.append({"role": "agent", "content": error_msg})
                return error_msg, False

        elif "[TOOL_CALL]" in response_text:
            try:
                # ИЗМЕНЕНО: Используем регулярное выражение для надежного извлечения JSON
                match = re.search(r'\{[\s\S]*\}', response_text)
                if not match:
                    raise ValueError("JSON-объект для вызова инструмента не найден в ответе модели.")

                tool_call_str = match.group(0)
                tool_call = json.loads(tool_call_str)
                tool_name = tool_call.get("tool")
                tool_params = tool_call.get("params", {})
                
                self.log(f"[{self.__class__.__name__}] Решил использовать '{tool_name}' с параметрами {tool_params}")
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
        """Стандартный метод для выполнения задачи агентом."""
        self.history = []
        final_result, _ = self.execute_step(task)
        return final_result