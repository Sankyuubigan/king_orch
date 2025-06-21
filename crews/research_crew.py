# crews/research_crew.py - УБРАНА ЛОКАЛЬНАЯ ЗАГРУЗКА КОНФИГА

import json
from agents.search_analyst_agent import SearchAnalystAgent
from agents.researcher_agent import ResearcherAgent
from agents.synthesis_agent import SynthesisAgent
from llama_cpp import Llama

class ResearchCrew:
    def __init__(self, llm_instance: Llama, tools_config: dict):
        self.llm = llm_instance
        self.trajectory = []
        # Конфиг теперь передается снаружи, а не загружается здесь
        self.tools_config = tools_config

    def _log(self, message):
        self.log_callback(message)
        self.trajectory.append(message)

    def run(self, topic: str, log_callback_from_engine):
        self.log_callback = log_callback_from_engine
        
        analyst = SearchAnalystAgent(self.llm, self.tools_config, self._log)
        researcher = ResearcherAgent(self.llm, self.tools_config, self._log)
        synthesizer = SynthesisAgent(self.llm, self._log)

        self._log(f"[Crew] Этап 1: Аналитик ищет источники по теме '{topic}'...")
        selected_urls = analyst.analyze_and_select_sources(topic)
        
        if not selected_urls:
            report = "Не удалось найти релевантные источники для исследования."
            self._log(f"[Crew] [ERROR] {report}")
            return {"final_result": report, "trajectory": self.trajectory}
        
        self._log(f"[Crew] Аналитик выбрал следующие URL для изучения: {selected_urls}")

        self._log("\n[Crew] Этап 2: Исследователь изучает каждый источник...")
        summaries = []
        for i, url in enumerate(selected_urls):
            self._log(f"  [Crew] -> Изучаю источник #{i+1}: {url}")
            summary = researcher.summarize_source(url)
            summaries.append(summary)
            self._log(f"  [Crew] <- Краткий вывод по источнику #{i+1} готов.")
        
        self._log("\n[Crew] Этап 3: Главный редактор пишет финальный отчет...")
        final_report = synthesizer.synthesize_report(topic, summaries)
        
        self._log("[Crew] Исследование завершено.")
        
        return {"final_result": final_report, "trajectory": self.trajectory}