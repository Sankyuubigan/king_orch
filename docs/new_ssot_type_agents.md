Задача, которую ты описываешь — одна из самых актуальных в промышленной разработке агентов (AI Agent Architecture). В индустрии эта проблема называется **Decomposition vs. Fusion** (декомпозиция против схлопывания шагов) или **Dynamic Execution Granularity** (динамическая гранулярность выполнения).

Профессионалы решают это разделением **описания логики задачи (Business Logic / Agent Intent)** и **движка её выполнения (Execution Engine / Strategy)**.

Ниже подробный разбор, как это правильно спроектировать, как организовать код на Rust и как решить дилемму с UI/UX.

---

### 1. Архитектурный паттерн: «Схлопывание пайплайна» (Pipeline / Prompt Fusion)

Вместо того чтобы писать два разных агента, ты описываешь задачу как цепочку **атомарных шагов (Atomic Steps)**. 

Каждый шаг содержит:
1. `Role/Intent`: Зачем нужен этот шаг (например, `AnalyzeUserIntent`, `FetchToolsData`, `FormatOutput`).
2. `Instruction`: Конкретная инструкция шага.
3. `ToolSet`: Инструменты, доступные на этом шаге.

А дальше твой движок компилирует этот граф в зависимости от режима:

```text
[Атомарные шаги]:  [Шаг 1: План] -> [Шаг 2: Поиск данных] -> [Шаг 3: Синтез ответа]
                           │
             ┌─────────────┴─────────────┐
             ▼                           ▼
    РЕЖИМ DECOMPOSED             РЕЖИМ FUSED (Схлопнутый)
    (Локальные / Слабые)        (Умные облачные / Быстрый режим)
  ┌─────────────────────┐      ┌─────────────────────────────┐
  │ LLM Call 1: Шаг 1   │      │ 1 LLM Call:                 │
  │ Context update      │      │ Объединенный System Prompt: │
  │ LLM Call 2: Шаг 2   │      │ "Сделай 1, затем 2,         │
  │ Tool Execution      │      │  и сформируй 3."            │
  │ LLM Call 3: Шаг 3   │      │ (LLM делает всё за раз,     │
  └─────────────────────┘      │  вызывая нужные tools пачкой)│
                               └─────────────────────────────┘
```

#### Как это работает на практике:
* **Для слабых / локальных моделей (`Decomposed`):** Ты шлешь запрос за запросом. Шаг 1 завершился -> его вывод идет в `history` для Шага 2 -> Шаг 2 вызывает один инструмент -> парсишь результат -> Шаг 3 генерирует ответ. Модель не захлебывается контекстом.
* **Для умных моделей (`Fused`):** Промпты Шага 1, 2 и 3 склеиваются в один мета-промпт с Chain-of-Thought инструкцией:
  > *"Тебе нужно решить задачу по шагам: 1) Спланируй..., 2) Вызови необходимые функции из списка, 3) На основе результатов выдай финальный ответ. Помести промежуточные размышления в блок `<thinking>`."*
  Умная модель (типа GPT-4o или Claude 3.5 Sonnet) сразу поймет план и, если надо, вернет **Parallel Tool Calls** (сразу 3-4 вызова инструментов за один ответ).

---

### 2. Уходим от дихотомии «Локальная vs Облачная»: Capability Profiles

Ты верно заметил: облачный `gpt-4o-mini` тупее локального `qwen2.5:72b` или `deepseek-r1`. Привязываться к транспорту (`llama.cpp` vs `OpenAI API`) нельзя.

Решение: создать структуру **`ModelProfile`** или **`CapabilityMatrix`**.

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum ReasoningTier {
    Low,      // Нужны атомарные шаги, 1 инструмент за раз (локалки 7B-14B)
    Medium,   // Справляется с простым ReAct, но сложные цепочки лучше делить
    High,     // Может в огромный промпт, parallel tool calls, саморефлексию (GPT-4o, Claude Sonnet)
}

#[derive(Debug, Clone)]
pub struct ModelProfile {
    pub id: String,
    pub reasoning_tier: ReasoningTier,
    pub supports_parallel_tools: bool,
    pub max_recommended_tools: usize, // Локалке лучше не давать больше 3-4 tools одновременно
    pub cost_sensitive: bool,         // Нужно ли экономить запросы
}
```

---

### 3. Реализация на Rust (Архитектура в Tauri)

В Rust это элегантно реализуется через трейты и паттерн **Стратегия (Strategy Pattern)**.

#### Шаг A: Атомарный шаг (Step)
```rust
use serde::{Deserialize, Serialize};

pub struct AgentStep {
    pub name: &'static str,
    pub instructions: String,
    pub tools_allowed: Vec<String>,
}

pub struct AgentWorkflow {
    pub steps: Vec<AgentStep>,
}
```

#### Шаг B: Компилятор пайплайна (Strategy)
```rust
#[derive(Debug, Clone, Copy)]
pub enum ExecutionStrategy {
    Decomposed, // Много шагов, экономия интеллекта, трата запросов
    Fused,      // Один/два шага, экономия запросов, высокая нагрузка на интеллект
}

pub struct LLMRequest {
    pub system_prompt: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDefinition>,
}

impl AgentWorkflow {
    pub fn compile(&self, strategy: ExecutionStrategy, user_input: &str) -> Vec<LLMPlanItem> {
        match strategy {
            ExecutionStrategy::Decomposed => {
                // Превращаем каждый шаг в отдельный раунд LLM
                self.steps.iter().map(|step| {
                    LLMPlanItem {
                        system_prompt: step.instructions.clone(),
                        tools: step.tools_allowed.clone(),
                        expect_intermediate_output: true,
                    }
                }).collect()
            }
            ExecutionStrategy::Fused => {
                // Склеиваем все шаги в один монолитный супер-промпт
                let mut combined_prompt = String::from("Выполни задачу, следуя этим фазам:\n");
                let mut all_tools = Vec::new();

                for (i, step) in self.steps.iter().enumerate() {
                    combined_prompt.push_str(&format!("{}. [Фаза {}]: {}\n", i + 1, step.name, step.instructions));
                    all_tools.extend(step.tools_allowed.clone());
                }
                combined_prompt.push_str("\nСначала продумай шаги, вызови нужные инструменты, затем дай итоговый ответ.");

                vec![LLMPlanItem {
                    system_prompt: combined_prompt,
                    tools: all_tools,
                    expect_intermediate_output: false,
                }]
            }
        }
    }
}
```

#### Шаг C: Исполнитель (Executor)
Исполнитель уже не знает, локальная модель или нет. Он просто берет список `LLMPlanItem` и крутит цикл:

```rust
pub struct AgentExecutor {
    client: Box<dyn LLMBackend>, // Трейт, который закрывает llama.cpp и OpenAI
}

impl AgentExecutor {
    pub async fn execute(
        &self, 
        plan: Vec<LLMPlanItem>, 
        context: &mut AgentContext
    ) -> Result<String, AgentError> {
        for step in plan {
            // Выполняем шаг (тут крутится ReAct-цикл для этого шага)
            let result = self.run_step_loop(&step, context).await?;
            context.append_step_result(step, result);
        }
        Ok(context.get_final_result())
    }
}
```

---

### 4. Как быть с инструментами (Tools) и экономией запросов?

Самое дорогое в облаке — это когда агент делает:
1. Запрос к LLM -> `tool_call`
2. Запрос к LLM -> `tool_call`
3. Запрос к LLM -> `answer`
*(3 запроса, 3 раза гоняется весь входящий контекст)*.

Как с этим бороться:
1. **Parallel Tool Calling (Параллельный вызов):**
   * В умных моделях OpenAI API умеет отдавать массив `tool_calls: [...]` в одном ответе.
   * Твой Rust-код должен исполнять их параллельно (через `futures::future::join_all`) и отдавать результат **всех вызовов в одном следующем сообщении**. Это сокращает цепочку с 5 запросов до 2!
2. **Tool Pruning (Фильтрация инструментов):**
   * Локальной модели нельзя отдавать 20 инструментов — она сойдет с ума.
   * Поэтому в `Decomposed` режиме каждый шаг открывает доступ *только* к 1-2 инструментам, нужным на этом этапе.
   * В облачной модели на `Fused` режиме ты вываливаешь все инструменты сразу.

---

### 5. Как оформить это в UI (Tauri / Десктоп)

Не перекладывай на пользователя технические термины вроде *«токены»*, *«шаги»* или *«гранулярность»*.

Сделай двухуровневую систему:

#### Вариант 1: Понятный переключатель режимов рассуждения (UX как в Perplexity / Cursor)
В интерфейсе чата сделать дропдаун или радиокнопки:
* ⚡ **«Быстрый (Экономный)»** — насильно включает `ExecutionStrategy::Fused`. Подходит для облака, минимум запросов, быстрый результат.
* 🧠 **«Пошаговый анализ (Глубокий)»** — включает `ExecutionStrategy::Decomposed`. В UI отображаются "мысли/шаги" агента в виде выпадающего аккордеона (`Step 1: Planning...`, `Step 2: Searching...`). Идеально для локалок и сложных задач.
* 🤖 **«Авто»** (по умолчанию) — берет стратегию из профиля выбранной модели:
  * Выбрана `llama.cpp` -> Автоматически ставится **Пошаговый**.
  * Выбран `gpt-4o` -> Автоматически ставится **Быстрый**.

#### Вариант 2: В настройках моделей (Model Settings)
В окне настроек моделей дай юзеру переопределить профиль модели:
* Чекбокс: `[x] Выполнять задачи пошагово (рекомендуется для слабых моделей)`
* Чекбокс: `[x] Разрешить параллельный вызов инструментов`

---

### Итоговый алгоритм для твоей архитектуры:

1. **Описывай агента как граф декларативных шагов**, где у каждого шага есть своя узкая роль и маленький список доступных tools.
2. **Используй паттерн Pipeline Compiler**:
   * Для слабых моделей он превращает граф в серию последовательных вызовов (с промежуточной очисткой контекста, чтобы локалка не упиралась в лимит токенов llama.cpp).
   * Для сильных моделей он "схлопывает" (fuses) системные промты в единую инструкцию, позволяя модели сделать всю цепочку рассуждений за 1-2 запроса.
3. **Абстрагируй бэкенд через единый трейт**: реализуй единый интерфейс поверх `llama-cpp-rs` (или HTTP к llama.cpp server) и официального OpenAI API формата.
4. **В UI дай один переключатель**: `Авто / Быстрый / Глубокий анализ`. Пользователь сам решит, хочет ли он потратить деньги/токены на облачной модели ради дотошного пошагового контроля, или ему достаточно ответа в 1 запрос.








Твой исходный YAML — это классический **низкоуровневый плоский граф (Flat Micro-Step DAG)**. 

В нём каждый «чих» мысли нейросети вынесен в отдельную ноду: `soma_translator`, `focus_keeper`, `decomposer`, `validator`, роутеры с проверками флагов `e1..e9` и т.д.
* Для **detailed** (локалки) это отлично: маленькая модель решает ровно одну микро-задачу за вызов.
* Для **fused** (облака) это катастрофа: 10–15 последовательных сетевых запросов, задержка в 30 секунд и прожигание денег на передаче дублирующегося контекста.

Чтобы объединить оба мира в едином пайплайне, граф делят на **Фазы (Phases / Super-Nodes)**. 

### Главный архитектурный секрет схлопывания:
Ты **не можешь** схлопнуть шаги сквозь пользователя (Human-in-the-loop). Если нужно спросить у юзера «хлопки/рукость» или «выбрать мишень», пайплайн обязан остановиться.
Поэтому твоя система естественным образом делится на **4 логические макро-фазы**, внутри которых:
* В режиме **detailed** — крутится твоя детальная цепочка сабагентов.
* В режиме **fused** — все сабагенты фазы объединяются в **один промпт с Chain-of-Thought (CoT)**, а промежуточные роутеры заменяются логикой рассуждения сильной модели.

---

### Обновлённый YAML: Архитектура «Phased Hierarchical Graph»

Вот как теперь выглядит твой пайплайн. Он описывает и **detailed**, и **fused** сценарии без дублирования сущностей.

```yaml
name: Психотерапевт
version: "2.0"

config:
  facts_file: facts.yaml
  default_llm_params: creative

# Точки входа и глобальные прерывания на человека (Human-In-The-Loop)
entry: phase_intake

phases:

  # --------------------------------------------------------------------------
  # ФАЗА 1: Входной фильтр и валидация запроса
  # --------------------------------------------------------------------------
  - id: phase_intake
    title: "Первичный анализ запроса"
    # Для FUSED: одна инструкция сильной модели (заменяет 2 запроса и роутер)
    fused_instruction: |
      Проанализируй ввод пользователя. Извлеки ключевые факты и определи, есть ли реальная 
      психологическая/соматическая проблема (has_problem: true/false).
      Если проблемы нет или запрос некорректен — выдай правила составления запроса (request_helper).
    
    # Для DETAILED: классические атомарные шаги
    detailed_steps:
      - id: extract_facts
        agent: llm_fact_extractor
        llm_params: strict
      - id: check_has_problem
        type: condition_check
        field: output.has_problem
        false_to: interrupt_invalid_request

    # Куда идти дальше
    on_success: phase_soma_and_focus
    interrupts:
      - id: interrupt_invalid_request
        type: human_response
        note_template: note_1785141025138 # Твоя нота с объяснением правил


  # --------------------------------------------------------------------------
  # ФАЗА 2: Соматика (НГМ) и Определение мишени
  # --------------------------------------------------------------------------
  - id: phase_soma_and_focus
    title: "Анализ соматики и выбор фокуса"
    
    fused_instruction: |
      1. Если у пользователя есть физическая боль/симптом — дай расшифровку по НГМ (soma_translator).
         ВНИМАНИЕ: Если для расшифровки критически не хватает биологической рукости, верни сигнал NEED_HANDEDNESS.
      2. Определи фокус проблемы (focus_keeper). Если это кластер проблем — декомпозируй на мишени (decomposer) 
         и верни сигнал NEED_TARGET_SELECTION со списком мишеней.
      3. Если проблема конкретна и понятна — переходи к формулированию контекста для валидации.
    
    detailed_steps:
      - id: check_somatic
        type: condition_router
        conditions: [{ field: somatic_presence, equals: new }]
        true_to: call_soma_translator
        false_to: call_focus_keeper
      - id: call_soma_translator
        agent: soma_translator
        task: "Пользователь описал боль. Дай ему расшифровку по НГМ."
      - id: check_need_handedness
        type: signal_router
        signal: "НУЖНА РУКОСТЬ"
        to: interrupt_ask_handedness
      - id: call_focus_keeper
        agent: focus_keeper
      - id: check_focus_signal
        type: signal_router
        cases:
          - key: "Кластер"
            to: call_decomposer
          - key: "НУЖЕН ВЫБОР МИШЕНИ"
            to: interrupt_ask_target
      - id: call_decomposer
        agent: decomposer

    on_success: phase_diagnosis
    interrupts:
      - id: interrupt_ask_handedness
        type: human_response
        note_template: note_1785123862689 # Тест с аплодисментами
      - id: interrupt_ask_target
        type: human_response
        note_template: note_ask_target    # Выбор номера мишени


  # --------------------------------------------------------------------------
  # ФАЗА 3: Глубокая диагностика элементов невроза (Самая тяжелая фаза!)
  # --------------------------------------------------------------------------
  - id: phase_diagnosis
    title: "Валидация элементов невроза и сбор недостающих данных"
    
    # ВОТ ЗДЕСЬ ОГРОМНАЯ ЭКОНОМИЯ ТОКЕНОВ В ОБЛАКЕ:
    # Вместо 4 вызовов (validator -> check -> shadow_worker/grounder/collector)
    # сильная модель делает всё это внутри своего скрытого CoT за 1 вызов!
    fused_instruction: |
      Оцени проблему по 9 элементам невроза (e1-e9).
      В блоке <thinking>:
      - Проверь наличие элементов e1-e9.
      - Если не хватает критических элементов (e3, e6, e8, e9):
        а) Примени оптику Теневого аспекта (shadow_worker) к ситуации.
        б) Смоделируй приземленные бытовые сценарии причин (grounder).
      Итогом сформируй полный валидированный диагностический профиль для подбора паттерна.
    
    detailed_steps:
      - id: call_validator
        agent: validator
        llm_params: strict
      - id: check_critical_elements
        type: condition_router
        conditions:
          - { field: validator_report.e3, equals: false }
          - { field: validator_report.e6, equals: false }
          - { field: validator_report.e8, equals: false }
          - { field: validator_report.e9, equals: false }
        logic: any
        true_to: branch_deep_investigation
        false_to: exit_to_healing
      # Ветки до-сбора (shadow_worker, grounder, data_collector)
      - id: branch_deep_investigation
        type: sub_graph
        entry: check_elements1
        # (Сюда уходят твои check_elements1, check_elements2, call_shadow_worker, call_grounder, call_collector)

    on_success: phase_healing


  # --------------------------------------------------------------------------
  # ФАЗА 4: Поиск паттерна и исцеление (Вызов инструментов)
  # --------------------------------------------------------------------------
  - id: phase_healing
    title: "Подбор паттерна и терапевтическое решение"
    
    tools:
      - ReadSection # Инструмент базы знаний
      
    fused_instruction: |
      1. На основе диагностического профиля определи номер типа паттерна невроза.
      2. ОБЯЗАТЕЛЬНО вызови инструмент `ReadSection` с этим номером, чтобы прочитать инструкцию из базы.
      3. Получив описание, переведи паттерн на контекст проблемы пользователя и выдай финальное терапевтическое решение.
    
    detailed_steps:
      - id: call_pattern_finder
        agent: pattern_finder_by_elements
      - id: call_healer
        agent: neuro_healer
        task: |
          Из отчёта pattern_finder_by_elements возьми номер типа паттерна.
          Выполни ШАГ 2 — прочитай описание инструментом ReadSection.
          Переведи паттерн на контекст проблемы.
        tools: [ReadSection]

    on_success: complete_pipeline
```

---

### Как твой Rust-бэкенд это исполняет:

В коде на Rust у тебя появляется `PipelineRunner`, который принимает `enum ExecutionStrategy { Detailed, Fused }`:

```rust
pub enum ExecutionStrategy {
    Detailed, // Локалка: крутит граф шаг за шагом
    Fused,    // Облако: схлопывает каждую фазу в 1 вызов к LLM
}

pub struct PhaseRunner {
    strategy: ExecutionStrategy,
}

impl PhaseRunner {
    pub async fn run_phase(&self, phase: &Phase, state: &mut PipelineState) -> PhaseResult {
        match self.strategy {
            ExecutionStrategy::Fused => {
                // 1. Берем phase.fused_instruction
                // 2. Делаем ВСЕГО ОДИН запрос в OpenAI API (передаем tools фазы, если есть)
                // 3. Сильная модель в JSON возвращает:
                //    - Либо сигнал прерывания (например: "NEED_HANDEDNESS") -> показываем UI ноту
                //    - Либо готовый результат фазы -> сразу переходим к следующей Phase!
                self.execute_fused_prompt(phase, state).await
            }
            ExecutionStrategy::Detailed => {
                // Крутим классический цикл:
                // step 1 -> step 2 -> router -> step 3 (через llama.cpp)
                self.execute_detailed_dag(&phase.detailed_steps, state).await
            }
        }
    }
}
```

### В чем профит для тебя как разработчика:
1. **Для облака (FUSED):** Вместо **12-14 сетевых запросов** весь этот огромный психотерапевтический путь проходит всего за **3-4 запроса** (по одному на каждую фазу, плюс вызов `ReadSection`). Экономия денег — до 70-80%, время отклика для пользователя сокращается в 3-5 раз.
2. **Для локалки (DETAILED):** Твоя `llama.cpp` на 8B-14B параметрах не сойдет с ума от необходимости помнить все 9 элементов невроза и правила НГМ одновременно. Она будет методично, маленькими порциями контекста жевать шаг за шагом, как и задумывалось в твоем оригинальном графе.
3. **Единый источник правды:** Тебе не нужно поддерживать два разных `.yaml` файла для облака и локалки. Логика прерываний (спросить рукость, выбрать мишень) едина для обоих режимов.






**Да, на 100% делай именно так! Твоя инженерная интуиция абсолютно правильная.**

Бросаться переписывать всех существующих агентов прямо сейчас — это классическая ловушка «великого переписывания» (The Great Rewrite), из-за которой разработка встанет на недели, полезут регрессионные баги и опустится мотивация.

В архитектуре ПО это называется **Graceful Degradation** (плавная деградация) и **Progressive Enhancement** (постепенное улучшение).

Вот как правильно и красиво организовать это в Rust и интерфейсе Tauri, чтобы не плодить спагетти-код:

---

### 1. Как сделать парсинг в Rust (через Serde)

В Rust благодаря `serde` это делается буквально в пару строк через `untagged` enum или через проверку полей.

Самый чистый вариант — **`#[serde(untagged)]`**:

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PipelineDefinition {
    // Новый формат (если есть поле "phases")
    Phased(PhasedPipeline),
    // Старый формат (если есть поле "nodes")
    Legacy(LegacyPipeline),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegacyPipeline {
    pub name: String,
    pub nodes: Vec<LegacyNode>,
    pub edges: Vec<LegacyEdge>,
    // ... твои старые поля
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhasedPipeline {
    pub name: String,
    pub phases: Vec<Phase>,
    // ... новые поля
}
```

Serde сам при чтении `.yaml` проверит структуру: если увидел `phases` — распарсит в `Phased`. Если не нашел — распарсит в твой старый привычный `Legacy`. Старые агенты **вообще не заметят разницы и продолжат работать как работали**.

---

### 2. Главный лайфхак архитектуры: Внутренняя нормализация (Internal Normalization)

Чтобы тебе в коде рантайма не пришлось писать костыли вида:
```rust
// ТАК ДЕЛАТЬ НЕ НАДО:
if is_legacy {
    run_old_engine();
} else {
    run_new_engine();
}
```

Ты делаешь **автоматический апкаст (Upcast)** старого пайплайна в новый в памяти прямо при загрузке:

> **Любой Legacy пайплайн — это просто Phased пайплайн, состоящий ровно из ОДНОЙ фазы, у которой нет инструкции для FUSED.**

```rust
impl LegacyPipeline {
    /// Превращаем старый пайплайн в Phase-структуру на лету
    pub fn into_virtual_phase(self) -> PhasedPipeline {
        PhasedPipeline {
            name: self.name,
            phases: vec![
                Phase {
                    id: "legacy_root_phase".to_string(),
                    title: "Полное выполнение (Legacy)".to_string(),
                    // Инструкции для схлопывания НЕТ, поэтому FUSED здесь недоступен:
                    fused_instruction: None, 
                    // Все старые ноды просто кладутся в detailed_steps:
                    detailed_steps: self.nodes,
                    edges: self.edges,
                }
            ],
        }
    }
}
```

**Что это дает?**
Твой новый рантайм-движок вообще **не знает про существование старого формата**! Он всегда исполняет `PhasedPipeline`. Старые агенты просто считаются агентами, у которых *«не прописан fused-режим»*.

---

### 3. Как это отразить в GUI (Tauri / Фронтенд)

В Tauri ты отправляешь в веб-интерфейс метаданные загруженного агента:

```rust
#[derive(Serialize)]
pub struct AgentMetaDto {
    pub name: String,
    pub supports_fused_mode: bool, // true, если есть phases И хотя бы в одной есть fused_instruction
}
```

А на фронтенде (React / Vue / Svelte) логика тумблера становится кристально понятной:

1. **Загружен старый агент (`supports_fused_mode == false`):**
   * Тумблер либо **скрыт**, либо **заблокирован (disabled)** в положении `Detailed`.
   * Рядом можно повесить серый бейдж/подсказку:  
     `[Пошаговый режим] (Классический пайплайн)`
   * Пользователь понимает: этот агент работает только по старой доброй детальной схеме.
2. **Загружен новый агент (`supports_fused_mode == true`):**
   * Тумблер становится активным:
     `[ ⚡ Быстрый (Fused) | 🧠 Пошаговый (Detailed) ]`
   * Пользователь (или ты сам) волен выбирать скорость/экономию токенов.

---

### Твой план действий (без выгорания):

1. **Оставь текущих агентов в покое.** Пусть они работают на 100% так, как работают сейчас.
2. В коде движка добавь поддержку распознавания: `nodes` -> режим `Detailed only`.
3. Возьми **только одного** агента (например, своего «Психотерапевта», так как он самый толстый и на нем больше всего будут видны выгоды схлопывания) и оформи его в новом формате `phases`.
4. Обкатай новый движок на нем. 
5. Остальных агентов переводи на фазы **только тогда**, когда тебе реально понадобится запускать их через OpenAI API дешевле и быстрее. Если агент простой (3-4 ноды) — его вообще можно не трогать, пусть живет в `Detailed` вечно.



# Терминология и названия

В агентных системах путаница между **жизненным циклом диалога (Multi-turn / Сессия)** и **жизненным циклом одного прогона (Single-turn / Пайплайн)** — главная причина архитектурной каши.

Слово «фаза» действительно перегружено: в психотерапии есть *«фаза сбора анамнеза»* и *«фаза интервенции»*, которые длятся десятками сообщений в чате! Плюс в твоем графе уже есть доменное слово `Кластер` (`key: Кластер -> call_decomposer`), поэтому и его использовать нельзя.

Чтобы раз и навсегда разделить эти уровни в коде, холсте и голове, используют стандартную трёхуровневую терминологию.

---

### Трёхуровневая модель (Честная терминология)

| Уровень | Что это | Как называем (RU / EN) | Где живёт |
| :--- | :--- | :--- | :--- |
| **1. Глобальный (Чат)** | Состояние сессии сквозь десятки сообщений («Анамнез», «Диагностика», «Лечение») | **Этап сессии** / `Session Stage` (или `Stage`) | В памяти сессии / БД диалога (`state.stage`) |
| **2. Локальный прогон** | Один цикл: сообщение юзера -> ответ агента | **Ход** / `Turn` (или `Run`) | В рантайме выполнения запроса |
| **3. Группировка на холсте** | Схлопываемые куски нод внутри одного `Turn` для FUSED-режима | **Блок** / `Block` (или **Группа** / `Group`) | В структуре YAML и на визуальном холсте |
| **4. Атомарный узел** | Конкретная микро-нода | **Узел** / `Node` | На холсте и в коде |

---

### Как это меняет YAML (Чисто и понятно)

Мы заменяем сомнительное слово `phase` на **`block`** (или **`group`**). 
В визуальных средах (Blender, Unreal Engine Blueprints, ComfyUI, ReactFlow) это стандарт: ноды объединяются в **Группы (Groups)** или **Блоки (Blocks)**.

В YAML это выглядит идеально понятно:

```yaml
name: Психотерапевт
entry: user_message

# ВЕРХНИЙ УРОВЕНЬ: Блоки схлопывания (для Fused-режима)
blocks:
  - id: intake_block
    title: "Первичный анализ"
    fused_instruction: "Извлеки факты. Если проблемы нет — верни request_helper."

  - id: soma_focus_block
    title: "Соматика и Мишень"
    fused_instruction: "Расшифруй соматику по НГМ. Выдели мишени..."

  - id: diagnosis_block
    title: "Диагностика элементов"
    fused_instruction: "Оцени e1-e9 через shadow_worker и grounder..."

  - id: healing_block
    title: "Исцеление"
    tools: [ReadSection]
    fused_instruction: "Найди паттерн, вызови ReadSection и дай ответ."

# ПЛОСКИЙ СПИСОК НОД: У каждой просто указан её block
nodes:
  - id: extract_facts
    type: llm_fact_extractor
    block: intake_block         # <-- Принадлежность к блоку
    ui_pos: { x: -1990, y: 493 }

  - id: check_has_problem
    type: condition_check
    block: intake_block         # <-- Тоже в intake_block
    ui_pos: { x: -1700, y: 495 }

  - id: call_soma_translator
    type: llm_worker
    block: soma_focus_block     # <-- Уже в следующем блоке
    ui_pos: { x: -561, y: 210 }
    
edges:
  # Обычные ребра
```

---

### Что это дает на всех уровнях:

1. **На визуальном холсте (Tauri GUI):**
   * Термин: **«Блок нод»** или **«Группа» (Node Group)**.
   * На холсте это буквально визуальная цветная рамка вокруг связанных нод.
   * Кликнул на заголовок блока — открылась боковая панель с полем `fused_instruction`.

2. **В кодовой базе Rust:**
   * `TurnExecutor` — движок исполнения одного хода (`Turn`).
   * `Block` / `NodeGroup` — структура метаданных схлопывания:
     ```rust
     pub struct Block {
         pub id: String,
         pub title: String,
         pub fused_instruction: Option<String>,
         pub tools: Vec<String>,
     }
     ```
   * А `Stage` (Этап сессии психотерапии) остаётся чистым для бизнес-логики:
     ```rust
     pub enum SessionStage {
         Greeting,
         Anamnesis,
         TargetWork,
         Closing,
     }
     ```

3. **Для твоего ИИ-кодера:**
   Ты снимаешь всю двусмысленность. Он чётко понимает:
   * **Stage** — это про сессию и долгосрочную память.
   * **Turn** — это один цикл от сообщения юзера до ответа.
   * **Group** — это граф-группировка нод внутри файла пайплайна для Fused/Detailed стратегии.

---

