//! Стенд: иерархическая (3-слойная) проверка 9 элементов невроза.
//!
//! # Гипотеза
//!
//! Плоская проверка 9 элементов одним вопросом даёт ложные срабатывания сильно:
//! 34 FP против 11 FN на `neurosis_base` и 45 FP против 8 FN на
//! `mixed_body_mental` (база Лайи, `gate_end_to_end_ornith.json`).
//!
//! Механизм FP измерен: когда оба полюса вопроса описывают страдание, эмбеддинги
//! сходятся, модель «не выбирает между полюсами и тянет в `true»»
//! (`tauri-plugin-system1/docs/LAYA_MODEL.md` §13.4.1, §16.1). Генеративная
//! модель на том же плоском вопросе уходит в другую сторону — почти всегда
//! `false` (recall 0.364, `tasks/08.10.26 …md` §5).
//!
//! Воронка меняет **форму запроса**, а не пороги: каждый слой спрашивает о
//! ПРИСУТСТВИИ в своём узком смысловом поле, где отрицание означает «не названо»,
//! а не «другой вид страдания». Слой 2 видит результат слоя 1, слой 3 — слоёв 1
//! и 2. Реляционная валидация между элементами **не мерялась ни разу**, так что
//! это неизмеренный рычаг, а не отвергнутый.
//!
//! # Что меряется и чем
//!
//! Ни промптов, ни грамматик в коде нет: стенд читает боевых агентов
//! (`agents/psychotherapist/backend/validator_*.md`) и их контракты сигналов
//! (`agents/psychotherapist/signals/root.schema.json`), то есть измеряет
//! ровно то, что поедет в бою. Промпт собирается теми же функциями, что и в
//! `execute_node` (`build_system_prompt`, `reports::build_injected_reports`).
//!
//! Вход — одно сообщение пациента, как в `gate_end_to_end_ornith.json`, на
//! котором замерена база Лайи. История переписки не подаётся сознательно:
//! сравнивать надо с той же базой.
//!
//! # Маршрут кейса — из боевого гейта, а не из золота
//!
//! Боевой граф не вызывает валидатор там, где факт-экстрактор вернул
//! `mental_presence == none` (`experiments/psychotherapist.yaml:319-322`).
//! Стенд повторяет это правило: такие кейсы проходят шорт-циркутом, все девять
//! элементов получают `false`, а сам факт и число шорт-циркутированных кейсов
//! публикуются в отчёте — иначе точность на соматике выдавала бы себя за заслугу воронки
//! (дефект, зафиксированный в `tasks/10.10.26 …md` §6).
//!
//! Решение гейта читается из измеренного артефакта `mental_gate::MentalGate`;
//! золото `expected` для маршрутизации не используется никогда.
//!
//! Метрика — общий счёт `verdict_contract::score_fields` и общая сводка
//! `probe_report::summarize` (те же, что у System-1 и у плоского System-2).
//!
//! # Гейты объявлены ДО прогона
//!
//! `tasks/11.10.26 Калибровка 3-слойного валидатора Орнитом.md` §5 (G0–G6).
//! Пороги не меняются после прогона. Правки критериев делались in-sample
//! (провалы найдены на тех же 63 кейсах), поэтому числа этого прогона —
//! измерение, но НЕ доказательство обобщающей способности (AGENTS.md §00.3.1).
//!
//! # Запуск
//!
//! ```text
//!   set TEST_MODEL_PATH=<путь к .gguf>
//!   set PROBE_DATASET=mixed_body_mental
//!   test.bat layered_validator_probe -- --ignored --nocapture
//! ```
//!
//! `PROBE_DATASET` пустой — все наборы. Отчёт перезаписывается после КАЖДОГО
//! кейса: прогон длиннее терпения терминала.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use serde_json::{json, Value};

use super::mental_gate::{self, MentalGate};
use super::probe_acceptance::AcceptanceBook;
use super::probe_call::{read_verdicts, CallFailure};
use super::probe_cases::{self, ProbeCase};
use super::probe_report::{routing_stats, summarize, upsert_row};
use super::reports::build_injected_reports;
use super::validator_layers::{self, ValidatorLayer};
use super::WorkflowRunner;
use crate::domain::agent_manager::load_agents;
use crate::domain::orchestrator::prompt::build_system_prompt;
use crate::domain::orchestrator::RequestMedia;
use crate::domain::signals::{
    build_signal_envelope_grammar_with_think_budget, load_all_signal_contracts,
    per_draft_budget_chars, required_field_count, SignalContract,
};
use crate::infra::config::load_config_early;
use crate::infra::{load_sampling_presets, ImageArtifactRegistry, ModelParams, SamplingPresets, SubCall, WriteOutside};
use crate::infra::{ChatMessage, LlmEngine};

/// Папка команды, чьи агенты и контракты образуют воронку.
const TEAM: &str = "psychotherapist";

/// Имя системы в книге приёмки (`test_cases/new_tests_for_validator/acceptance_gates.json`).
///
/// Пороги, метрики, направления и диагностический кейс лежат в книге, а не в
/// коде: гейт — утверждение о приёмке, он обязан быть проверяемым другим
/// (core rules §1.11) и объявленным до прогона (AGENTS.md §00.3.1).
const ACCEPTANCE_SYSTEM: &str = "layered_validator_funnel";

/// Маршрут кейса: слои вызваны или бой их не вызывает.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Route {
    /// Валидатор вызван, все три слоя отработали.
    Funnel,
    /// `mental_presence == none`: боевой граф валидатор не вызывает.
    ShortCircuit,
}

impl Route {
    fn as_str(self) -> &'static str {
        match self {
            Route::Funnel => "funnel",
            Route::ShortCircuit => "short_circuit",
        }
    }
}

/// Один слой воронки, готовый к вызову: контракт, грамматика и собранный промпт.
struct LayerRuntime {
    layer: ValidatorLayer,
    contract: SignalContract,
    grammar: String,
    system_prompt: String,
    /// Граница размышления из frontmatter агента: попадает в отчёт, чтобы
    /// строки разных конфигураций не смешались в одной сводке.
    think_budget_chars: Option<usize>,
    /// `disable_reasoning:` из frontmatter агента — единственный источник
    /// флага (тот же, что читает боевая нода). Тоже попадает в отчёт: строки,
    /// снятые с размышлением и без него, — разные замеры.
    disable_reasoning: bool,
}

struct FunnelHarness {
    engine: LlmEngine,
    layers: Vec<LayerRuntime>,
    sampling_presets: SamplingPresets,
    model_params: ModelParams,
    max_gen_tokens: usize,
    agents_dir: std::path::PathBuf,
    mcp_servers_dir: std::path::PathBuf,
    bins_dir: std::path::PathBuf,
    workspace_root: std::path::PathBuf,
    /// Решения боевого гейта `mental_presence`, снятые тем же факт-экстрактором
    /// на тех же кейсах. Маршрут кейса берётся отсюда, а не из золота.
    gate: MentalGate,
    /// Все девять элементов воронки — из `validator_layers.yaml`, чтобы набор
    /// шорт-циркутированных вердиктов не расходился с объявлением слоёв.
    all_elements: Vec<String>,
}

/// Собирает готовые к вызову слои из боевых файлов агентов — **без движка**.
///
/// Вынесено отдельно от `FunnelHarness::start`, чтобы механику воронки
/// (состав, контракты, грамматики, сборка промптов) можно было проверить
/// обычным юнит-тестом, без 646-мегабайтной модели.
fn prepare_layers(
    agents_dir: &Path,
    max_gen_tokens: usize,
) -> Result<Vec<LayerRuntime>, String> {
    let team_dir = agents_dir.join(TEAM);
    let declared = validator_layers::load(&team_dir)?;
    let contracts = load_all_signal_contracts(&team_dir.join("signals"));
    // Расхождение состава слоёв и контрактов — ошибка ДО инференса, иначе стенд
    // мерял бы форму ответа, которой в бою не бывает.
    validator_layers::validate_against_contracts(&declared, &contracts)?;

    let agents = load_agents(agents_dir)?;
    let mut layers = Vec::with_capacity(declared.len());
    for layer in declared {
        let agent = agents
            .iter()
            .find(|agent| agent.id == layer.agent)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "агент слоя '{}' не найден в agents/ ({})",
                    layer.agent,
                    team_dir.display()
                )
            })?;
        if !agent.tools.iter().any(|tool| tool == "emit_signal") {
            return Err(format!(
                "агент слоя '{}' без tools: [\"emit_signal\"] — он не сможет отдать сигнал",
                layer.agent
            ));
        }
        let contract = contracts
            .get(&layer.agent)
            .cloned()
            .ok_or_else(|| format!("нет контракта сигнала у слоя '{}'", layer.agent))?;
        // Граница черновика — РАСЧЁТ от бюджета генерации, а не заголовок агента.
        // Замер 11.10.26 без границы: 7 слоёв из 63 не дошли до конверта, ответы
        // по 6851–8786 символов, общая точность 0.7083 против 0.7847 у старой
        // конфигурации. Промпт с просьбой «сокращай черновики» модель проигнорировала,
        // поэтому граница обязана быть в грамматике.
        let drafts = required_field_count(&contract.value_schema);
        if drafts == 0 {
            return Err(format!(
                "у контракта слоя '{}' нет обязательных полей — грамматика потребовала бы \
                 ноль черновиков и модель не смогла бы дойти до конверта",
                layer.agent
            ));
        }
        let draft_budget_chars = per_draft_budget_chars(max_gen_tokens, drafts);
        let grammar = build_signal_envelope_grammar_with_think_budget(&contract, draft_budget_chars);
        if draft_budget_chars.is_none() {
            return Err(format!(
                "бюджет генерации {} токенов не вмещает ни одного черновика слоя '{}' \
                 ({} штук) — увеличьте max_gen_tokens",
                max_gen_tokens, layer.agent, drafts
            ));
        }
        // Базовый промпт слоя — боевой .md с развёрнутым INCLUDE плюс честный
        // протокол. Хвост (плагины, инжект предыдущих слоёв, блок лимита) добавляется
        // перед каждым вызовом: инжект зависит от того, что уже ответили слои выше.
        let system_prompt = build_system_prompt(
            &agent,
            &[],
            false,
            &[],
            max_gen_tokens,
            true,
            false,
            false,
            false,
        );
        layers.push(LayerRuntime {
            layer,
            contract,
            grammar,
            system_prompt,
            // В отчёт идёт ИМЕННО та граница, что попала в грамматику. Запись
            // значения из frontmatter здесь соврала бы: конфигурация замера
            // обязана совпадать с тем, на чём реально считалось.
            think_budget_chars: draft_budget_chars,
            disable_reasoning: agent.disable_reasoning,
        });
    }
    Ok(layers)
}

impl FunnelHarness {
    /// Поднимает движок и собирает слои воронки из боевых файлов.
    ///
    /// Маршрут кейса берётся из измеренного артефакта боевого гейта по ИМЕНИ
    /// МОДЕЛИ: золото (`expected`) для маршрутизации не используется — это
    /// ответчик, и подставить его сюда означало бы измерять себя на себе.
    fn start(model_path: &str) -> Result<Self, String> {
        let root = probe_cases::project_root();
        let agents_dir = root.join("agents");

        let app_config = load_config_early();
        let max_gen_tokens = app_config.max_gen_tokens as usize;
        let layers = prepare_layers(&agents_dir, max_gen_tokens)?;

        let model_label = Path::new(model_path)
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_string())
            .unwrap_or_else(|| model_path.to_string());
        let gate = MentalGate::load(&model_label)?;
        let book = AcceptanceBook::load(&root)?;
        book.system(ACCEPTANCE_SYSTEM)?;

        let all_elements: Vec<String> = validator_layers::covered_elements(
            &layers
                .iter()
                .map(|l| l.layer.clone())
                .collect::<Vec<validator_layers::ValidatorLayer>>(),
        )
        .into_iter()
        .collect();

        let engine = LlmEngine::local(
            crate::infra::llm::LlamaEngine::new(
                &probe_cases::resolve_engine_dir(),
                model_path,
                app_config.context_size,
                false,
                false,
                0,
                &|message| eprintln!("[engine] {}", message),
                |_| {},
            )
            .map_err(|error| format!("движок не запустился: {}", error))?,
        );

        Ok(Self {
            engine,
            layers,
            sampling_presets: load_sampling_presets(&root),
            model_params: app_config
                .model_params
                .get(model_path)
                .cloned()
                .unwrap_or_default(),
            max_gen_tokens,
            agents_dir,
            mcp_servers_dir: root.join("src-tauri").join("mcp_servers"),
            bins_dir: root.join("src-tauri").join("bin"),
            workspace_root: root,
            gate,
            all_elements,
        })
    }

    /// Один кейс через всю воронку: слои по порядку, каждый следующий видит
    /// сигналы предыдущих тем же текстом, что и боевая нода.
    fn run_case(&mut self, case: &ProbeCase) -> Result<FunnelRun, CallFailure> {
        let mut verdicts: BTreeMap<String, bool> = BTreeMap::new();
        let mut messages: Vec<ChatMessage> = Vec::new();
        let mut steps: Vec<Value> = Vec::new();

        let mental = self
            .gate
            .decision(case)
            .map_err(|error| CallFailure::new(error, Vec::new()))?
            .to_string();
        if mental == mental_gate::MENTAL_NONE {
            // Боевой граф валидатор здесь не вызывает. Элементы не «проверены и
            // не найдены» — их не проверяли; для счёта это честный `false`,
            // публикуемый в отчёте как `route: short_circuit` + число таких кейсов.
            let verdicts = self
                .all_elements
                .iter()
                .map(|element| (element.clone(), false))
                .collect();
            return Ok(FunnelRun {
                verdicts,
                steps,
                route: Route::ShortCircuit,
                mental_presence: mental,
            });
        }

        // Данные слоя копируются ДО цикла: `generate` берёт `&mut self`, а сам
        // слой лежит в `self.layers` — без копии компилятор не даёт совместить
        // два этих borrow, и пришлось бы дробить состояние ради цикла.
        let plan: Vec<LayerCall> = self
            .layers
            .iter()
            .map(|layer| LayerCall {
                agent: layer.layer.agent.clone(),
                elements: layer.layer.elements.clone(),
                context_from: layer.layer.context_from.clone(),
                contract: layer.contract.clone(),
                grammar: layer.grammar.clone(),
                system_prompt: layer.system_prompt.clone(),
                disable_reasoning: layer.disable_reasoning,
            })
            .collect();

        for (index, layer) in plan.iter().enumerate() {
            let started = std::time::Instant::now();
            let mut step = json!({
                "layer": layer.agent,
                "elements": layer.elements,
            });

            // Инжект собирается функцией, которую вызывает и `execute_node`, а
            // хвост промпта — общей `finalize_system_prompt`: стенд видит ровно
            // тот же текст, что увидит боевая нода.
            let injected = build_injected_reports(
                &super::parser::NodeDef {
                    inject_response: Some(layer.context_from.clone()),
                    ..super::parser::NodeDef::default()
                },
                &messages,
            );
            let prompt = crate::domain::orchestrator::prompt::finalize_system_prompt(
                &layer.system_prompt,
                &layer.agent,
                &injected,
            );

            let (text, reasoning) =
                match self.generate(case, &layer.agent, &prompt, &layer.grammar, layer.disable_reasoning) {
                    Ok(pair) => pair,
                    Err(error) => {
                        // Отказ движка: ответа нет вообще, но номер слоя и причина
                        // обязаны попасть в отчёт — иначе кейс выглядит как обычный
                        // «нераспарсен» и диагностика теряет место отказа.
                        step["error"] = json!(error);
                        step["ms"] = json!(started.elapsed().as_millis());
                        steps.push(step);
                        return Err(CallFailure::new(
                            format!("слой {} не ответил: {}", layer.agent, error),
                            steps,
                        ));
                    }
                };
            step["chars"] = json!(text.chars().count());
            step["reasoning_chars"] = json!(reasoning.chars().count());
            step["raw"] = json!(text);

            match read_verdicts(&layer.contract, &text, &reasoning) {
                Ok(signal_value) => {
                    for (element, flag) in &signal_value {
                        verdicts.insert(element.clone(), *flag);
                    }
                    step["signal"] = json!(signal_value);
                    // Сигнальное сообщение — ровно то, что кладёт в `messages[]`
                    // оркестратор (`dispatch.rs::handle_emit_signal`).
                    let content = serde_json::json!({ layer.contract.key.clone(): signal_value });
                    messages.push(ChatMessage {
                        id: Some(format!("probe_{}_{}", case.id, index)),
                        msg_type: "signal".to_string(),
                        content: content.to_string(),
                        sub_calls: None,
                        author: Some(layer.agent.clone()),
                        model: None,
                        time_sec: None,
                        attachments: None,
                        phase: Some(2),
                    });
                }
                Err(error) => {
                    step["error"] = json!(error);
                    // Воронка без слоя не доезжает до конца: следующий слой получил бы
                    // контекст из воздуха. Стопимся здесь, кейс уходит в ошибку.
                    step["ms"] = json!(started.elapsed().as_millis());
                    steps.push(step);
                    return Err(CallFailure::new(
                        format!("слой {} не ответил: {}", layer.agent, error),
                        steps,
                    ));
                }
            }
            step["ms"] = json!(started.elapsed().as_millis());
            steps.push(step);
        }

        Ok(FunnelRun {
            verdicts,
            steps,
            route: Route::Funnel,
            mental_presence: mental,
        })
    }

    fn generate(
        &mut self,
        case: &ProbeCase,
        layer: &str,
        prompt: &str,
        grammar: &str,
        disable_reasoning: bool,
    ) -> Result<(String, String), String> {
        let mut all_sub_calls: Vec<SubCall> = Vec::new();
        let mut msg_counter: u32 = 0;
        let runner = WorkflowRunner {
            engine: &self.engine,
            agents: &[],
            workflows: &[],
            log_cb: |message: String| eprintln!("[probe] {}", message),
            status_cb: |_: String, _: u8| {},
            subcall_cb: |_: &SubCall| {},
            max_gen_tokens: self.max_gen_tokens,
            model_params: &self.model_params,
            format_type: "Auto",
            cancel_flag: Arc::new(AtomicBool::new(false)),
            mcp_servers_dir: &self.mcp_servers_dir,
            bins_dir: &self.bins_dir,
            grammars_dir: &self.agents_dir,
            agents_dir: &self.agents_dir,
            all_sub_calls: &mut all_sub_calls,
            msg_counter: &mut msg_counter,
            stream_meta: Arc::new(std::sync::Mutex::new(Default::default())),
            sampling_presets: &self.sampling_presets,
            prompt_log: None,
            session_id: format!("layered_validator_{}_{}", case.dataset, layer),
            workspace_root: self.workspace_root.clone(),
            write_root: self.workspace_root.join(".agents_workspace"),
            request_media: Arc::new(RequestMedia::empty()),
            image_artifacts: Arc::new(ImageArtifactRegistry::new()),
            write_outside: WriteOutside::Deny,
        };
        let params = runner.resolve_llm_params(&Some("strict".to_string()), &None, None);
        runner
            .call_llm_direct(
                prompt,
                &case.text,
                &params,
                &format!("probe:layered_validator:{}", layer),
                Some(grammar.to_string()),
                // Флаг из frontmatter агента (`disable_reasoning:`) — тот же, что
                // читает боевая нода: у reasoning-моделей размышление идёт
                // отдельным каналом, грамматика его не ограничивает, и весь
                // `n_predict` может уйти в think вместо конверта.
                disable_reasoning,
            )
            .map_err(|error| format!("LLM не отработала: {}", error))
    }
}

/// Результат прогона одного кейса через всю воронку.
struct FunnelRun {
    verdicts: BTreeMap<String, bool>,
    steps: Vec<Value>,
    /// Воронка отработала или кейс прошёл шорт-циркут по боевому гейту.
    route: Route,
    /// Значение факта `mental_presence`, которым выбран маршрут.
    mental_presence: String,
}

/// Всё, что нужно слою для одного вызова: копия данных слоя, снятая до цикла
/// вызовов, где нужен `&mut self` на движок.
struct LayerCall {
    agent: String,
    elements: Vec<String>,
    context_from: Vec<String>,
    contract: SignalContract,
    grammar: String,
    system_prompt: String,
    disable_reasoning: bool,
}

#[test]
#[ignore]
fn layered_validator_probe() {
    let model_path = std::env::var("TEST_MODEL_PATH")
        .expect("Set TEST_MODEL_PATH to a GGUF file path");
    let dataset_filter = std::env::var("PROBE_DATASET").unwrap_or_default();

    let all_cases = probe_cases::load_all_cases().expect("кейсы прочитаны");
    assert!(!all_cases.is_empty(), "ни одного кейса — замер безвреден");

    let model_label = Path::new(&model_path)
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_string())
        .unwrap_or_else(|| model_path.clone());

    let mut harness = FunnelHarness::start(&model_path).expect("стенд не поднялся");
    let acceptance = AcceptanceBook::load(&probe_cases::project_root())
        .expect("книга приёмки прочитана");
    acceptance
        .system(ACCEPTANCE_SYSTEM)
        .unwrap_or_else(|error| panic!("{}", error));
    let config = ReportConfig::of(&harness.layers, harness.gate.source(), harness.max_gen_tokens);
    println!(
        "\n########## воронка: {} (граница размышления: {:?} симв., размышление \
         выключено: {:?}) ##########",
        config.layer_names.join(" → "),
        config.think_budget_chars,
        config.disable_reasoning
    );

    let report_path = probe_cases::results_dir().join(format!(
        "layered_validator_{}.json",
        model_label
    ));
    let mut rows = config
        .read_rows(&report_path)
        .unwrap_or_else(|error| panic!("{}", error));

    let cases: Vec<&ProbeCase> = all_cases
        .iter()
        .filter(|case| dataset_filter.is_empty() || dataset_filter == case.dataset)
        .collect();

    for (index, case) in cases.iter().enumerate() {
        let started = std::time::Instant::now();
        let mut row = json!({
            "dataset": case.dataset,
            "id": case.id,
            "text": case.text,
            "expected": case.expected.clone().unwrap_or_default(),
            "has_labels": case.expected.is_some(),
        });

        match harness.run_case(case) {
            Ok(run) => {
                row["steps"] = Value::Array(run.steps);
                row["unparsed_layers"] = json!(0);
                row["route"] = json!(run.route.as_str());
                row["mental_presence"] = json!(run.mental_presence);
                match super::verdict_contract::score_fields(
                    case.expected.as_ref(),
                    &run.verdicts,
                ) {
                    Some((correct, fields, tp, tn, fp, fn_)) => {
                        row["verdicts"] = json!(run.verdicts);
                        row["correct"] = json!(correct);
                        row["fields"] = json!(fields);
                        row["tp"] = json!(tp);
                        row["tn"] = json!(tn);
                        row["fp"] = json!(fp);
                        row["fn"] = json!(fn_);
                        println!(
                            "[{}/{}] {} {} поля {}/{} ({} мс)",
                            index + 1,
                            cases.len(),
                            case.dataset,
                            case.id,
                            correct,
                            fields,
                            started.elapsed().as_millis()
                        );
                    }
                    None => {
                        // Набор без разметки (метафоры): интересует сам факт
                        // «элементы найдены», поля не считаются.
                        let any_true = run.verdicts.values().any(|flag| *flag);
                        row["verdicts"] = json!(run.verdicts);
                        row["any_element_found"] = json!(any_true);
                        println!(
                            "[{}/{}] {} {} элементов найдено: {}",
                            index + 1,
                            cases.len(),
                            case.dataset,
                            case.id,
                            any_true
                        );
                    }
                }
            }
            Err(failure) => {
                // Прогон останавливается на первом неответившем слое: слоя,
                // который не дал сигнал, в воронке ровно один.
                row["unparsed_layers"] = json!(1);
                row["error"] = json!(failure.message);
                // Сырые ответы слоёв, включая отказавший, — единственный материал
                // для разбора отказа. Без них отчёт фиксирует симптом
                // («конверт не найден») и молчит о причине.
                row["steps"] = json!(failure.steps);
                if let Some(expected) = &case.expected {
                    let (correct, fields, tp, tn, fp, fn_) =
                        super::verdict_contract::score_failed_case(expected);
                    row["correct"] = json!(correct);
                    row["fields"] = json!(fields);
                    row["tp"] = json!(tp);
                    row["tn"] = json!(tn);
                    row["fp"] = json!(fp);
                    row["fn"] = json!(fn_);
                }
                eprintln!(
                    "[probe] ПРОВАЛ {} {}: {}",
                    case.dataset,
                    case.id,
                    failure.message
                );
            }
        }

        upsert_row(&mut rows, row);
        // Отчёт после каждого кейса: длинный прогон нельзя терять целиком.
        write_report(&report_path, &model_path, &rows, &config, &acceptance);
    }

    let summary = summarize(&rows);
    let correct_fields = summary
        .pointer("/overall/correct")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let gates = evaluate_gates(&acceptance, &summary, &rows, correct_fields);
    write_report(&report_path, &model_path, &rows, &config, &acceptance);
    println!(
        "\n=== воронка ({}) ===\n{}\nГейты:\n{}\nОтчёт: {}",
        model_label,
        serde_json::to_string_pretty(&summary).expect("json"),
        serde_json::to_string_pretty(&gates).expect("json"),
        report_path.display()
    );
}

fn write_report(
    path: &Path,
    model_path: &str,
    rows: &[Value],
    config: &ReportConfig,
    book: &AcceptanceBook,
) {
    let summary = summarize(rows);
    let correct_fields = summary
        .pointer("/overall/correct")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let report = json!({
        "note": "Иерархическая воронка 9 элементов невроза: 3 агента-слоя из \
                 agents/psychotherapist/backend/, контракты — из signals/root.schema.json, \
                 состав — из database/validator_layers.yaml. Вход — одно сообщение пациента \
                 (как в gate_end_to_end_ornith.json). Маршрут кейса — из боевого гейта \
                 mental_presence (артефакт fact_extractor_mental_*.json), золото для \
                 маршрутизации не используется. Гейты объявлены ДО прогона в \
                 test_cases/new_tests_for_validator/acceptance_gates.json (пороги — из \
                 tasks/11.10.26 Калибровка 3-слойного валидатора Орнитом.md §5). Правки \
                 критериев in-sample, поэтому числа этого прогона — измерение, но НЕ \
                 доказательство обобщающей способности (AGENTS.md §00.3.1).",
        "model": model_path,
        "system": book.system_label(ACCEPTANCE_SYSTEM).unwrap_or(ACCEPTANCE_SYSTEM),
        "layers": config.layer_names,
        "think_budget_chars": config.think_budget_chars,
        "max_gen_tokens": config.max_gen_tokens,
        "disable_reasoning": config.disable_reasoning,
        "gate_source": config.gate_source,
        "routing": routing_stats(rows, "funnel"),
        "summary": summary,
        // Гейты в артефакте: без них вердикт остаётся в консоли прогона, и
        // через месяц его уже нечем проверить.
        "gates": evaluate_gates(book, &summary, rows, correct_fields),
        "rows": rows,
    });
    super::probe_call::write_report(path, &report);
}

/// Конфигурация замера: она же хранится в отчёте.
///
/// Смешивание строк от разных конфигураций в одном отчёте — ложь в цифрах: после
/// правки грамматики или бюджета старые строки относятся к другой системе. Поэтому
/// при расхождении конфигураций прогон останавливается, а не дописывает кейсы в
/// чужой отчёт. Тот же запрет касается файла гейта: другой артефакт — другая
/// маршрутизация, то есть другая система.
struct ReportConfig {
    layer_names: Vec<String>,
    think_budget_chars: Vec<Option<usize>>,
    disable_reasoning: Vec<bool>,
    gate_source: String,
    /// Бюджет генерации, из которого выведены границы черновиков. Без него в
    /// отчёте нечем объяснить, почему границы именно такие: при смене
    /// `max_gen_tokens` в конфиге пользователя числа меняются, и старый отчёт
    /// перестаёт быть сравнимым, хотя все прочие поля совпадают.
    max_gen_tokens: usize,
}

impl ReportConfig {
    fn of(layers: &[LayerRuntime], gate_source: &Path, max_gen_tokens: usize) -> Self {
        Self {
            layer_names: layers.iter().map(|l| l.layer.agent.clone()).collect(),
            think_budget_chars: layers
                .iter()
                .map(|l| l.think_budget_chars)
                .collect(),
            disable_reasoning: layers.iter().map(|l| l.disable_reasoning).collect(),
            gate_source: gate_source.display().to_string(),
            max_gen_tokens,
        }
    }

    fn as_json(&self) -> Value {
        json!({
            "layers": self.layer_names,
            "think_budget_chars": self.think_budget_chars,
            "disable_reasoning": self.disable_reasoning,
            "gate_source": self.gate_source,
            "max_gen_tokens": self.max_gen_tokens,
        })
    }

    /// Отчёт прошлого прогона читается только если конфигурация совпала.
    fn read_rows(&self, path: &Path) -> Result<Vec<Value>, String> {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Ok(Vec::new());
        };
        let report: Value = serde_json::from_str(&text)
            .map_err(|error| format!("{} не разобран: {}", path.display(), error))?;
        let previous = json!({
            "layers": report.get("layers").cloned().unwrap_or(Value::Null),
            "think_budget_chars": report.get("think_budget_chars").cloned().unwrap_or(Value::Null),
            "disable_reasoning": report.get("disable_reasoning").cloned().unwrap_or(Value::Null),
            "gate_source": report.get("gate_source").cloned().unwrap_or(Value::Null),
            "max_gen_tokens": report.get("max_gen_tokens").cloned().unwrap_or(Value::Null),
        });
        if previous != self.as_json() {
            return Err(format!(
                "отчёт {} заполнен другой конфигурацией (было: {}, сейчас: {}). \
                 Удалите файл — иначе сводка смешает два разных замера",
                path.display(),
                previous,
                self.as_json()
            ));
        }
        Ok(report
            .get("rows")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    }
}

/// Гейты из книги приёмки. `verdict` считается по правилам, объявленным ДО
/// прогона: замена плоской проверки обсуждается только при зелёных гейтах.
///
/// Пороги, направления и диагностический кейс лежат в
/// `test_cases/new_tests_for_validator/acceptance_gates.json`, а не в константах
/// этого файла. Расчёт — общий с [`AcceptanceBook`] с плоским стендом, поэтому
/// два замера не могут оказаться посчитаны по разным правилам.
fn evaluate_gates(
    book: &AcceptanceBook,
    summary: &Value,
    rows: &[Value],
    correct_fields: i64,
) -> Value {
    // Неразобранные СЛОИ, а не кейсы: у воронки три вызова на кейс.
    let unparsed_layers: usize = rows
        .iter()
        .filter_map(|row| row.get("unparsed_layers").and_then(Value::as_u64))
        .map(|count| count as usize)
        .sum();

    let mut out = book
        .evaluate(ACCEPTANCE_SYSTEM, summary, correct_fields, unparsed_layers)
        .unwrap_or_else(|error| panic!("книга приёмки не разобрана: {}", error));

    // Диагностика объявленного кейса. Гейтом не является: поля `passed` у блока
    // нет, поэтому в общий вердикт не попадает.
    if let Some(case_id) = book.diagnostic_case(ACCEPTANCE_SYSTEM) {
        let verdicts = rows
            .iter()
            .find(|row| row.get("id").and_then(Value::as_str) == Some(case_id))
            .and_then(|row| row.get("verdicts"))
            .cloned();
        out.as_object_mut().expect("evaluate вернул объект").insert(
            "diagnostic_target_case".to_string(),
            json!({
                "case": case_id,
                "note": "диагностика, не приёмка",
                "verdicts": verdicts,
            }),
        );
    }

    let entries = out
        .as_object()
        .expect("evaluate вернул объект")
        .values()
        .filter_map(Value::as_object)
        .filter(|entry| entry.contains_key("passed"))
        .map(|entry| entry.get("passed") == Some(&json!(true)))
        .collect::<Vec<_>>();
    let all_passed = entries.iter().all(|passed| *passed);
    let gates_total = entries.len();
    out.as_object_mut()
        .expect("evaluate вернул объект")
        .insert(
            "verdict_all_gates_passed".to_string(),
            json!({
                "all_passed": all_passed,
                "gates_total": gates_total,
                "gates_green": entries.iter().filter(|passed| **passed).count(),
            }),
        );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layers() -> Vec<LayerRuntime> {
        let agents_dir = probe_cases::project_root().join("agents");
        prepare_layers(&agents_dir, 2048).expect("слои собираются из боевых агентов")
    }

    /// Механика воронки проверяется БЕЗ модели: если слои, грамматики или
    /// промпты не собрались, об этом дешевле узнать до часового прогона.
    #[test]
    fn funnel_layers_are_ready_without_model() {
        let layers = layers();
        assert_eq!(layers.len(), 3, "воронка объявлена тремя слоями");

        let names: Vec<&str> = layers
            .iter()
            .map(|layer| layer.layer.agent.as_str())
            .collect();
        assert_eq!(
            names,
            vec!["validator_surface", "validator_facade", "validator_core"]
        );

        for layer in &layers {
            let agent_id = layer.layer.agent.as_str();
            // Грамматика берётся из контракта: все поля слоя в ней присутствуют.
            for element in &layer.layer.elements {
                assert!(
                    layer.grammar.contains(&format!("\\\"{}\\\"", element)),
                    "в грамматике слоя {} нет ключа {}",
                    agent_id,
                    element
                );
            }
            assert!(
                layer.grammar.contains(&layer.contract.key),
                "в грамматике слоя {} нет его сигнального ключа {}",
                agent_id,
                layer.contract.key
            );
            // Промпт собран из боевого .md с развёрнутым INCLUDE: критерии
            // элементов обязаны быть видны модели, иначе слой судит без правил.
            assert!(
                layer.system_prompt.contains("evidence_test"),
                "в промпте слоя {} нет критериев элементов",
                agent_id
            );
            assert!(
                layer.system_prompt.contains(&layer.contract.key),
                "в промпте слоя {} не назван его сигнальный ключ {}",
                agent_id,
                layer.contract.key
            );
        }
    }

    /// Второй слой обязан видеть сигнал первого тем же текстом, что и боевая
    /// нода: иначе «контекст предыдущего слоя» — красивый комментарий.
    #[test]
    fn later_layer_prompt_contains_previous_layer_signal() {
        let layers = layers();
        let previous = &layers[0];
        let current = &layers[1];

        let messages = vec![ChatMessage {
            id: Some("probe_test".to_string()),
            msg_type: "signal".to_string(),
            content: serde_json::json!({ previous.contract.key.clone(): json!({"e1": true}) }).to_string(),
            sub_calls: None,
            author: Some(previous.layer.agent.clone()),
            model: None,
            time_sec: None,
            attachments: None,
            phase: Some(2),
        }];

        let injected = build_injected_reports(
            &super::super::parser::NodeDef {
                inject_response: Some(current.layer.context_from.clone()),
                ..super::super::parser::NodeDef::default()
            },
            &messages,
        );
        assert!(
            injected.contains(previous.layer.agent.as_str()),
            "в инжекте нет предыдущего слоя: {}",
            injected
        );
        assert!(
            injected.contains("\"e1\":true"),
            "в инжекте нет самих вердиктов предыдущего слоя: {}",
            injected
        );

        // Стенд собирает итоговый промпт общей функцией — значит он видит ровно
        // то, что увидит боевая нода, вместе с блоком критического лимита.
        let prompt = crate::domain::orchestrator::prompt::finalize_system_prompt(
            &current.system_prompt,
            &current.layer.agent,
            &injected,
        );
        assert!(
            prompt.contains("\"e1\":true"),
            "вердикты предыдущего слоя не попали в промпт слоя {}",
            current.layer.agent
        );
        assert!(
            prompt.contains("[КРИТИЧЕСКОЕ ОГРАНИЧЕНИЕ]"),
            "в промпте стенда нет блока лимита, который есть в бою"
        );
    }

    /// Конверт `emit_signal` разбирается тем же парсером, что и в бою, и
    /// проверяется против контракта: неполный value обязан быть провалом.
    #[test]
    fn layer_signal_is_parsed_and_validated() {
        let layers = layers();
        let surface = &layers[0];
        let envelope = "<think>разбор</think> {\"tool\":\"emit_signal\",\"arguments\":{\"key\":\
            \"validator_surface\",\"value\":{\"e1\":true,\"e2\":false,\"e6\":false,\"e7\":true}}}";
        let verdicts = read_verdicts(&surface.contract, envelope, "")
            .expect("конверт разобран");
        assert_eq!(verdicts.get("e1"), Some(&true));
        assert_eq!(verdicts.get("e7"), Some(&true));
        assert_eq!(verdicts.get("e2"), Some(&false));
        assert_eq!(verdicts.len(), 4);

        // Конверт в канале размышлений — тоже валидный ответ (именно так ведёт себя
        // reasoning-модель, оставляя content пустым).
        let in_reasoning = read_verdicts(&surface.contract, "", envelope)
            .expect("конверт в reasoning_content разобран");
        assert_eq!(in_reasoning.get("e1"), Some(&true));

        let incomplete = "{\"tool\":\"emit_signal\",\"arguments\":{\"key\":\
            \"validator_surface\",\"value\":{\"e1\":true}}}";
        let error =
            read_verdicts(&surface.contract, incomplete, "").expect_err("неполный value — провал");
        assert!(error.contains("требует все поля контракта"), "{}", error);

        let free_text = "<think>разбор</think> просто текст без конверта";
        let error = read_verdicts(&surface.contract, free_text, "").expect_err("без конверта — провал");
        assert!(error.contains("нет конверта"), "{}", error);

        // Пусто в обоих каналах — провал с честным перечислением каналов.
        let error = read_verdicts(&surface.contract, "", "").expect_err("пусто в обоих каналах");
        assert!(error.contains("content: пусто") && error.contains("reasoning: пусто"), "{}", error);
    }

    /// Слой без контекста не должен получать чужой раздел инжекта: первый слой
    /// воронки работает на «голом» тексте пациента.
    #[test]
    fn first_layer_has_no_context() {
        let layers = layers();
        assert!(
            layers[0].layer.context_from.is_empty(),
            "первый слой не должен получать контекст"
        );
        assert_eq!(layers[2].layer.context_from.len(), 2, "третий слой видит оба предыдущих");
    }

/// Свой черновик на каждый элемент: число слотов в грамматике обязано равняться
/// числу элементов слоя. Рассинхрон означал бы либо отказ модели перейти к
/// конверту (лишний слот), либо молчаливый пропуск обдумывания (нехватка).
#[test]
    fn one_draft_per_declared_element() {
        let layers = layers();
        assert!(!layers.is_empty());
        for layer in &layers {
            let count = layer.layer.elements.len();
            let expected = format!("drafts-block ::= draft{{{}}}", count);
            assert!(
                layer.grammar.contains(&expected),
                "у слоя {} {} элементов, а грамматика ждёт другое число черновиков: нет '{}'",
                layer.layer.agent,
                count,
                expected
            );
        }
    }

    /// Граница черновика выводится из бюджета генерации, а не берётся из
    /// frontmatter агента: пользовательский `max_gen_tokens` — единственный
    /// источник. В заголовках слоёв цифры быть не должно, иначе агент зафиксирует
    /// границу, не согласующуюся с тем, сколько токенов реально выдаёт движок.
    #[test]
    fn agents_declare_no_think_budget() {
        for layer in layers() {
            assert_ne!(
                layer.think_budget_chars, None,
                "у слоя {} граница не выведена из бюджета",
                layer.layer.agent
            );
        }
        let agents_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("корень проекта")
            .join("agents");
        let agents = crate::domain::agent_manager::load_agents(&agents_dir).expect("агенты прочитаны");
        for id in ["validator_surface", "validator_facade", "validator_core"] {
            let agent = agents
                .iter()
                .find(|agent| agent.id == id)
                .unwrap_or_else(|| panic!("агент слоя {} не найден", id));
            assert_eq!(
                agent.think_budget_chars, None,
                "в frontmatter слоя {} снова появилась цифра границы: {:?}",
                id,
                agent.think_budget_chars
            );
        }
    }

/// Границы черновиков обязаны помещаться в бюджет генерации вместе с
    /// конвертом. Замер 11.10.26 без границ: 7 слоёв из 63 не дошли до конверта
    /// (ответы 6851–8786 символов), общая точность 0.7083 против 0.7847.
    /// Инвариант проверяется на числах, а не на словах: сумма границ всех
    /// черновиков слоя, переведённая в токены, обязана помещаться в
    /// `max_gen_tokens` вместе с резервом под конверт.
    #[test]
    fn draft_budgets_fit_the_generation_budget() {
        const BUDGET: usize = 2048;
        for layer in layers() {
            let drafts = layer.layer.elements.len();
            let chars = layer
                .think_budget_chars
                .expect("граница черновика обязана быть выведена из бюджета");
            // Обратный перевод грамматики в токены. Проверяем, что при самом
            // пессимистичном для нас соотношении символов к токенам (2.0)
            // черновики всё ещё помещаются вместе с резервом под конверт.
            let tokens = chars.div_ceil(2);
            assert!(
                drafts * tokens <= BUDGET.saturating_sub(256),
                "у слоя {} {} черновиков по {} символов: это {} токенов, бюджет {} \
                 токенов вместе с резервом под конверт не вмещает",
                layer.layer.agent,
                drafts,
                chars,
                drafts * tokens,
                BUDGET
            );
        }
    }

    /// Порядок элементов, названный в промпте слоя, обязан совпадать с порядком
    /// обязательных полей контракта: грамматика требует N черновиков подряд, и
    /// если промпт велит «в порядке полей `value`», а поля идут иначе, модель
    /// получает противоречивую инструкцию.
    #[test]
    fn prompt_element_order_matches_contract() {
        for layer in layers() {
            let positions: Vec<usize> = layer
                .layer
                .elements
                .iter()
                .map(|element| {
                    layer.system_prompt.find(element.as_str()).unwrap_or_else(|| {
                        panic!(
                            "в промпте слоя {} вообще нет элемента {} — черновик «по одному \
                             на элемент» не с чем соотнести",
                            layer.layer.agent, element
                        )
                    })
                })
                .collect();
            for (index, element) in layer.layer.elements.iter().enumerate() {
                if index > 0 && positions[index] <= positions[index - 1] {
                    panic!(
                        "в промпте слоя {} элемент {} назван раньше {}, хотя контракт требует \
                         порядка {:?} — инструкция «черновик по одному на элемент» противоречива",
                        layer.layer.agent,
                        element,
                        layer.layer.elements[index - 1],
                        layer.layer.elements
                    );
                }
            }
        }
    }

    /// Грамматика, которую РЕАЛЬНО получает `llama-server`, выгружается на диск.
    ///
    /// Движок отвергает грамматику своим парсером, и все тесты Rust при этом
    /// остаются зелёными: 11.10.26 прогон отпал за 18 секунд на всех 63 кейсах с
    /// `failed to parse grammar`, и ни один тест этого не заметил. Единственная
    /// проверка, которая ловит такое, — скармлинг файла самому движку:
    /// `llama-server --grammar-file dump_<слой>.gbnf --model <несуществующий>`.
    ///
    /// Файл берётся из `prepare_layers`, а не собирается заново: иначе на диск
    /// попадёт строка, которой в бою не бывает, и проверка будет бесполезной.
    #[test]
    fn dump_funnel_grammars_for_engine_check() {
        for layer in layers() {
            let out = probe_cases::project_root()
                .join("test")
                .join("laya_probe")
                .join(format!("dump_{}.gbnf", layer.layer.agent));
            std::fs::write(&out, &layer.grammar).expect("записать грамматику");
            println!(
                "=== {} -> {} (граница черновика {:?}) ===\n{}",
                layer.layer.agent,
                out.display(),
                layer.think_budget_chars,
                layer.grammar
            );
        }
    }

    /// Книга приёмки читается из данных, а не из кода, и её содержимое
    /// фиксируется тестом. Раньше те же числа стояли константой `GATES` рядом
    /// с расчётом, поэтому «гейт объявлен до прогона» проверялся самим собой.
    ///
    /// Тест читает НАСТОЯЩУЮ книгу из репозитория: расхождение с заданием
    /// поймается здесь, а не в отчёте стенда.
    #[test]
    fn acceptance_book_declares_funnel_gates_before_the_run() {
        let book = AcceptanceBook::load(&probe_cases::project_root())
            .expect("книга приёмки прочитана");
        let acceptance = book
            .system(ACCEPTANCE_SYSTEM)
            .expect("воронка объявлена в книге приёмки");

        let gates: Vec<(&str, f64)> = acceptance
            .gates
            .iter()
            .map(|gate| (gate.id.as_str(), gate.threshold))
            .collect();
        assert_eq!(
            gates,
            vec![
                ("O1_overall_vs_baseline", 0.7222),
                ("O2_somatic_vs_baseline", 0.963),
                ("O3_neurosis_specificity_floor", 0.3678),
                ("O4_mixed_specificity_floor", 0.3284),
                ("O5_metaphor_accuracy_vs_baseline", 0.4444),
                ("R1_overall_no_regression", 0.7847),
            ],
            "пороги приёмки разошлись с tasks/11.10.26 …md §9"
        );
        // Все пороги — минимумы: «не хуже базовой Система-1 и не хуже своего
        // прошлого прогона». Обратного направления в объявлении быть не может.
        assert!(
            acceptance
                .gates
                .iter()
                .all(|gate| gate.bound == super::super::probe_acceptance::Bound::AtLeast),
            "в приёмке воронки появился гейт «не больше» — его надо обосновать"
        );
        // Гейт по метафорам обязан требовать измерения: иначе «потеряно 0» и
        // «метафоры не мерили» дают одно и то же число.
        let metaphor = acceptance
            .gates
            .iter()
            .find(|gate| gate.id == "O5_metaphor_accuracy_vs_baseline")
            .expect("гейт по метафорам объявлен");
        assert_eq!(
            metaphor.measured_on.as_deref(),
            Some("body_metaphor"),
            "гейт по метафорам без measured_on проходит вслепую"
        );
    }

    /// Расчёт гейтов на данных: зелёный там, где порог взят, красный там, где
    /// набора нет. Проверяет не числа (они в тесте выше), а то, что книга
    /// действительно применяется к сводке.
    #[test]
    fn gates_are_computed_from_the_book() {
        let book = AcceptanceBook::load(&probe_cases::project_root()).expect("книга прочитана");
        let rows = vec![json!({
            "dataset": "neurosis_base", "id": "c",
            "correct": 200, "fields": 234, "tp": 119, "tn": 40, "fp": 24, "fn": 51,
            "unparsed_layers": 0,
        })];
        let summary = super::super::probe_report::summarize(&rows);
        let gates = evaluate_gates(&book, &summary, &rows, 200);
        // В этой выборке только невроз, поэтому считается ровно он.
        // O3: specificity 40/64 = 0.625 — выше базовой 0.3678, гейт зелёный.
        assert_eq!(gates["O3_neurosis_specificity_floor"]["passed"], json!(true));
        // O1 и R1: 200/234 = 0.8547 — выше обоих порогов.
        assert_eq!(gates["O1_overall_vs_baseline"]["passed"], json!(true));
        assert_eq!(gates["R1_overall_no_regression"]["passed"], json!(true));
        // O2 и O4: наборов нет — метрики `null`, гейт обязан быть красным.
        assert_eq!(gates["O2_somatic_vs_baseline"]["passed"], json!(false));
        assert_eq!(gates["O4_mixed_specificity_floor"]["passed"], json!(false));
        // O5: набор метафор не мерили — гейт не должен проходить вслепую.
        assert_eq!(gates["O5_metaphor_accuracy_vs_baseline"]["measured"], json!(false));
        assert_eq!(gates["O5_metaphor_accuracy_vs_baseline"]["passed"], json!(false));
        assert_eq!(gates["G0_unparsed"]["passed"], json!(true));
    }

    /// Один неразобранный слой из трёх на кейс — уже отказ. Гейт считает СЛОИ,
    /// а не кейсы: воронка делает три вызова на кейс, и «0 из 63» против
    /// «0 из 189» — разные числа.
    #[test]
    fn unparsed_gate_counts_layers_not_cases() {
        let book = AcceptanceBook::load(&probe_cases::project_root()).expect("книга прочитана");
        let rows = vec![json!({
            "dataset": "neurosis_base", "id": "c",
            "correct": 200, "fields": 234, "tp": 119, "tn": 40, "fp": 24, "fn": 51,
            "unparsed_layers": 1,
        })];
        let summary = super::super::probe_report::summarize(&rows);
        let gates = evaluate_gates(&book, &summary, &rows, 200);
        assert_eq!(gates["G0_unparsed"]["actual"], json!(1));
        assert_eq!(gates["G0_unparsed"]["passed"], json!(false));
    }

    /// Гейт по метафорам теперь СЧИТАЕТСЯ по настоящей разметке, а не «сколько
/// кейсов потеряно». Прежний вариант был ловушкой: он зеленел и на наборе,
/// который вообще не мерили. Проверяем, что гейт требует измерения и считает
/// именно точность по полям.
#[test]
    fn metaphor_gate_requires_measurement_and_scores_accuracy() {
        let book = AcceptanceBook::load(&probe_cases::project_root()).expect("книга прочитана");
        let gates_for = |metaphor_cases: usize, correct_per_case: usize| {
            let mut rows = vec![json!({
                "dataset": "somatic_only", "id": "s1",
                "correct": 9, "fields": 9, "tp": 0, "tn": 9, "fp": 0, "fn": 0,
                "unparsed_layers": 0,
            })];
            for index in 0..metaphor_cases {
                rows.push(json!({
                    "dataset": "body_metaphor", "id": format!("m{}", index),
                    "verdicts": {"e1": true},
                    "correct": correct_per_case, "fields": 9,
                    "tp": correct_per_case, "tn": 9 - correct_per_case, "fp": 0,
                    "fn": 9 - correct_per_case,
                    "unparsed_layers": 0,
                }));
            }
            let summary = super::super::probe_report::summarize(&rows);
            let correct = summary
                .pointer("/overall/correct")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            evaluate_gates(&book, &summary, &rows, correct)
        };

        // 15 кейсов, по 4 верных поля из 9 — точность 0.4444, ровно на пороге базы.
        let at = gates_for(15, 4);
        assert_eq!(at["O5_metaphor_accuracy_vs_baseline"]["measured"], json!(true));
        assert_eq!(
            at["O5_metaphor_accuracy_vs_baseline"]["actual"],
            json!(4.0 / 9.0)
        );
        assert_eq!(at["O5_metaphor_accuracy_vs_baseline"]["passed"], json!(true));

        // Ни одного кейса метафор: метрики нет — гейт обязан быть красным,
        // а не «нулевым и потому зелёным».
        let none = gates_for(0, 0);
        assert_eq!(none["O5_metaphor_accuracy_vs_baseline"]["measured"], json!(false));
        assert_eq!(none["O5_metaphor_accuracy_vs_baseline"]["passed"], json!(false));
    }

    /// Кейс, прошедший шорт-циркут, обязан получить все девять элементов из
    /// объявления слоёв — иначе набор вердиктов разошёлся бы с воронкой.
    #[test]
    fn short_circuit_verdicts_cover_every_declared_element() {
        let layers = layers();
        let all_elements: Vec<String> =
            validator_layers::covered_elements(&layers.iter().map(|l| l.layer.clone()).collect::<Vec<_>>())
                .into_iter()
                .collect();
        let gate = MentalGate::from_report(
            &serde_json::json!({"rows": [{"dataset": "somatic_only", "id": "s1", "mental_presence": "none"}]}),
            Path::new("test-gate.json"),
        )
        .expect("гейт прочитан");
        let case = ProbeCase {
            dataset: "somatic_only",
            id: "s1".to_string(),
            text: "болит спина".to_string(),
            expected: None,
        };
        assert!(!gate.validator_is_called(&case).expect("решение гейта"));
        let verdicts: BTreeMap<String, bool> = all_elements
            .iter()
            .map(|element| (element.clone(), false))
            .collect();
        assert_eq!(verdicts.len(), 9, "воронка объявляет девять элементов");
        assert!(verdicts.values().all(|flag| !flag));
    }
}
