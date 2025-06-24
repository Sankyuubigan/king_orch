# crews/research_crew.py - ИСПРАВЛЕНА СИГНАТУРА МЕТОДА RUN

import json
from agents.search_analyst_agent import SearchAnalystAgent
from agents.researcher_agent import ResearcherAgent
from agents.synthesis_agent import SynthesisAgent
from llama_cpp import Llama

class ResearchCrew:
    def __init__(self, llm_instance: Llama, tools_config: dict):
        self.llm = llm_instance
        self.trajectory = []
        self.tools_config = tools_config
        self.update_callback = None # Добавлено для консистентности

    def _log(self, message):
        self.log_callback(message)
        self.trajectory.append(message)
        
    def _send_update(self, update_type: str, data: any):
        """Отправляет структурированное сообщение в UI."""
        if self.update_callback:
            self.update_callback({"type": update_type, "data": data})

    # ИЗМЕНЕНО: Добавлен третий аргумент `update_callback_from_engine` для унификации
    def run(self, topic: str, log_callback_from_engine, update_callback_from_engine=None):
        self.log_callback = log_callback_from_engine
        self.update_callback = update_callback_from_engine # Сохраняем для использования в _send_update

        analyst = SearchAnalystAgent(self.llm, self.tools_config, self._log)
        researcher = ResearcherAgent(self.llm, self.tools_config, self._log)
        synthesizer = SynthesisAgent(self.llm, self._log)

        self._log(f"[Crew] Этап 1: Аналитик ищет источники по теме '{topic}'...")
        selected_urls = analyst.analyze_and_select_sources(topic)
        
        if not selected_urls:
            report = "Не удалось найти релевантные источники для исследования."
            self._log(f"[Crew] [ERROR] {report}")
            final_result = {"final_result": report, "trajectory": self.trajectory}
            self._send_update("final_result", final_result)
            return final_result
        
        self._log(f"[Crew] Аналитик выбрал следующие URL для изучения: {selected_urls}")

        self._log("\n[Crew] Этап 2: Исследователь изучает каждый источник...")
        summaries = []
        for i, url in enumerate(selected_urls):
            self._log(f"  [Crew] -> Изучаю источник #{i+1}: {url}")
            summary = researcher.summarize_source(url)
            summaries.append(summary)
            self._log(f"  [Crew] <- Краткий вывод по источнику #{i+1} готов.")
        
        self._log("\n[Crew] Этап 3: Главный редактор пишет финальный отчет...")
        final_report_text = synthesizer.synthesize_report(topic, summaries)
        
        self._log("[Crew] Исследование завершено.")
        
        final_result = {"final_result": final_report_text, "trajectory": self.trajectory}
        self._send_update("final_result", final_result)
        
        return final_result