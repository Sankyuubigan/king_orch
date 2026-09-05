# План исследований: Two-Phase Generation в одном запросе

## Основной вопрос
Как open-source LLM проекты реализуют "двухфазную генерацию" в одном запросе:
- Фаза 1: Модель генерирует свободно (рассуждения/мышление)
- Фаза 2: После рассуждений модель генерирует структурированный вывод (JSON)
- Обе фазы происходят в одном вызове инференса

## Подтемы для исследования

### 1. llama.cpp: thinking + structured output
- Как llama.cpp обрабатывает `reasoning_format` + grammar в одном запросе
- Есть ли механизм "grammar trigger" или "lazy grammar", который активирует грамматику после think блока
- Как работает `llama_sampler` для переключения между фазами
- Файлы для проверки: `llama.cpp/src/llama-grammar.cpp`, `llama.cpp/src/llama-sampler.cpp`

### 2. vLLM: thinking + JSON schema
- Поддерживает ли vLLM thinking + JSON schema в одном запросе
- Как реализовано переключение между reasoning и structured output
- Есть ли аналог "grammar after think block"

### 3. Ollama: поддержка паттерна
- Поддерживает ли Ollama двухфазную генерацию
- Как Ollama взаимодействует с llama.cpp для этого функционала

### 4. Специфические реализации
- Поиск "llama.cpp thinking grammar single request"
- Поиск "llama.cpp reasoning_format json_schema"
- Issues/PRs о "grammar after think block" или "reasoning + structured output"

## Ожидаемая информация
- Конкретные файлы и код из репозиториев
- PRs и issues с обсуждениями
- Примеры использования API
- Ограничения и известные проблемы

## Источники
- https://github.com/ggml-org/llama.cpp
- https://github.com/vllm-project/vllm
- https://github.com/ollama/ollama
- Документация и issue трекеры проектов