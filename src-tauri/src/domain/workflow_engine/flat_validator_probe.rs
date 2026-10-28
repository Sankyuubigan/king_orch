//! Стенд: плоская форма проверки 9 элементов невроза — ОДИН вызов, девять черновиков.
//!
//! # Зачем этот стенд
//!
//! Замер 11.10.26 §10.3 поднял точность трёхслойной воронки с 0.7083 до 0.7847
//! (затем 0.8009) **двумя** изменениями сразу: черновик-на-элемент вместо одного
//! черновика на слой И граница длины черновика в грамматике. Он не отвечает на
//! вопрос, чем именно воронка лучше плоской формы: разделением на слои или тем,
//! что граница черновика вообще появилась.
//!
//! Этот стенд — недостающая ячейка. Здесь ровно одна переменная: **число вызовов**.
//! Всё остальное — критерии (`element_validation_rules.yaml` через `<<INCLUDE>>`),
//! грамматика (`signals::build_signal_envelope_grammar_with_think_budget`),
//! функция границы (`signals::per_draft_budget_chars`), модель, семплирование
//! `strict`, боевой гейт `mental_presence` и счёт (`verdict_contract`).
//!
//! # Почему не старый стенд плоского Орнита
//!
//! Измеренные там 0.697 — не замер боевой ноды `call_validator` (этот стенд
//! вынесен в `test/laya_probe/experiments/ornith_validator_probe.rs`). Он брал
//! промпт `verdict_contract::build_verdict_prompt()` из критериев Лайи
//! (исторический `element_validation_rules_system1.yaml`, файл удалён) и ждал
//! голый `{e1..e9}` без конверта `emit_signal`. Это другая система; сравнивать её с
//! воронкой нельзя. Здесь промпт — боевой `.md` агента, а ответ — конверт
//! `emit_signal` с ключом `validator_report`, ровно как у `call_validator`.
//!
//! # Две ячейки
//!
//! `per_draft_budget_chars(2048, 9)` = **398** символов на черновик против **896**
//! у слоя из четырёх. То есть плоская форма получает втрое меньше места под
//! размышление не из-за формы, а из-за числа черновиков. Поэтому:
//!
//! - **F1** — граница «как есть» (398). Отделяет форму от границы.
//! - **F2** — граница выровнена (10752 / 9 = **1195**): суммарно столько же места
//!   под размышление, сколько у воронки. Отделяет разделение на слои от бюджета.
//!
//! Без F2 утверждение «три слоя лучше» недоказуемо (AGENTS.md §00.3): деградация F1
//! объяснялась бы не числом вызовов, а втрое меньшим местом под черновики.
//!
//! # Гейты объявлены ДО прогона
//!
//! `tasks/11.10.26 Плоский валидатор против трёхслойной воронки.md` §5. Пороги
//! после прогона не двигаются. Ограничения L1–L4 там же: главное — промпты не
//! идентичны, и замер in-sample.
//!
//! # Запуск
//!
//! ```text
//!   set TEST_MODEL_PATH=D:\nn\models\llm\uncen\Ornith-1.5-9B-uncensored.i1-IQ4_NL.gguf
//!   set PROBE_DATASET=somatic_only
//!   test.bat flat_validator_probe -- --ignored --nocapture
//! ```
//!
//! `PROBE_DATASET` пуст — все наборы. Отчёт перезаписывается после КАЖДОГО кейса.
//! Уже измеренные кейсы пропускаются: `read_rows` возвращает только строки этой
//! конфигурации, а `upsert_row` заменяет кейс по `dataset` + `id`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use serde_json::{json, Value};

use super::mental_gate::{self, MentalGate};
use super::probe_acceptance::AcceptanceBook;
use super::probe_call::{read_verdicts, CallFailure};
use super::probe_cases::{self, ProbeCase};
use super::probe_report::{routing_stats, summarize, upsert_row};
use super::validator_layers;
use super::verdict_contract::{score_fields, score_failed_case};
use super::WorkflowRunner;
use crate::domain::agent_manager::load_agents;
use crate::domain::orchestrator::prompt::{build_system_prompt, finalize_system_prompt};
use crate::domain::orchestrator::RequestMedia;
use crate::domain::signals::{
    build_signal_envelope_grammar_with_think_budget, load_all_signal_contracts, per_draft_budget_chars,
    required_field_count, SignalContract,
};
use crate::infra::config::load_config_early;
use crate::infra::{
    load_sampling_presets, ImageArtifactRegistry, LlmEngine, ModelParams, SamplingPresets, SubCall,
    WriteOutside,
};

/// Агент плоской формы. Живёт рядом со слоями, но в воронку не входит: состав
/// слоёв объявляет `experiments/validator_layers.yaml`, и плоский агент там не значится.
const FLAT_AGENT: &str = "validator_flat";

/// Идентификатор системы в книге приёмки
/// (`test_cases/new_tests_for_validator/acceptance_gates.json`).
const ACCEPTANCE_SYSTEM: &str = "flat_validator";

/// Суммарное место под размышление у трёхслойной воронки при текущем бюджете.
///
/// **Считается из боевой конфигурации слоёв**, а не хранится числом: F2 существует
/// ровно затем, чтобы плоская форма получила столько же текста для размышления,
/// сколько воронка. Если состав слоёв или `max_gen_tokens` изменятся, «выровненный»
/// бюджет обязан измениться вместе с ними — иначе ячейка F2 тихо перестанет быть
/// выровненной, а её имя продолжит врать.
fn funnel_total_draft_chars(agents_dir: &Path, max_gen_tokens: usize) -> Result<usize, String> {
    let team_dir = agents_dir.join("psychotherapist");
    let layers = validator_layers::load(&team_dir)?;
    let contracts = load_all_signal_contracts(&team_dir.join("signals"));
    let mut total = 0usize;
    for layer in &layers {
        let contract = contracts
            .get(&layer.agent)
            .ok_or_else(|| format!("у слоя '{}' нет контракта сигнала", layer.agent))?;
        let drafts = required_field_count(&contract.value_schema);
        let chars = per_draft_budget_chars(max_gen_tokens, drafts).ok_or_else(|| {
            format!(
                "бюджет {} токенов не вмещает ни одного черновика слоя '{}' ({} штук)",
                max_gen_tokens, layer.agent, drafts
            )
        })?;
        total += chars * drafts;
    }
    Ok(total)
}

/// Соответствие состава контракта порядку черновиков в промпте — единственный
/// инвариант формы плоского агента, и он проверяется тестом, а не константой
/// в коде стенда.

/// Плоский агент, готовый к вызову: контракт, грамматика и собранный промпт.
struct FlatRuntime {
    contract: SignalContract,
    grammar: String,
    system_prompt: String,
    /// Граница, реально попавшая в грамматику. Не из frontmatter: граница
    /// считается из бюджета генерации, и в отчёт идёт именно применённая.
    think_budget_chars: usize,
    disable_reasoning: bool,
    elements: Vec<String>,
}

/// Собирает готовый к вызову плоский слой из боевых файлов агента — **без движка**.
///
/// Отдельная функция ради юнит-теста: весь механизм (контракт, границы, промпт,
/// соответствие порядка черновиков контракту) проверяется без 646-мегабайтной модели.
fn prepare_flat(
    agents_dir: &Path,
    max_gen_tokens: usize,
    draft_budget_override: Option<usize>,
) -> Result<FlatRuntime, String> {
    let team_dir = agents_dir.join("psychotherapist");
    let contracts = load_all_signal_contracts(&team_dir.join("signals"));
    let contract = contracts
        .get(FLAT_AGENT)
        .cloned()
        .ok_or_else(|| format!("нет контракта сигнала у '{}'", FLAT_AGENT))?;

    let agents = load_agents(agents_dir)?;
    let agent = agents
        .iter()
        .find(|agent| agent.id == FLAT_AGENT)
        .cloned()
        .ok_or_else(|| {
            format!(
                "агент '{}' не найден в agents/ ({})",
                FLAT_AGENT,
                team_dir.display()
            )
        })?;
    if !agent.tools.iter().any(|tool| tool == "emit_signal") {
        return Err(format!(
            "агент '{}' без tools: [\"emit_signal\"] — он не сможет отдать сигнал",
            FLAT_AGENT
        ));
    }

    // Черновик на каждый обязательный элемент контракта. Сколько их — берётся из
    // контракта, а не из промпта: иначе порядок и число черновиков разъезжаются
    // с формой ответа, и модель получает невыполнимое задание.
    let drafts = required_field_count(&contract.value_schema);
    if drafts == 0 {
        return Err(format!(
            "у контракта '{}' нет обязательных полей — грамматика потребовала бы ноль \
             черновиков и модель не смогла бы дойти до конверта",
            FLAT_AGENT
        ));
    }
    // Состав элементов контракта — единственный источник: стенд не объявляет
    // список `e1..e9` своим, иначе расхождение с контрактом осталось бы незамеченным.
    // Порядок же (промпт велит писать элементы подряд в порядке `required`, а
    // грамматика ждёт ровно его) проверяет `prompt_draft_order_matches_contract`.
    let contract_elements = required_bool_keys(&contract)?;

    let think_budget_chars = match draft_budget_override {
        Some(budget) => budget,
        None => per_draft_budget_chars(max_gen_tokens, drafts).ok_or_else(|| {
            format!(
                "бюджет генерации {} токенов не вмещает ни одного черновика ({} штук) — \
                 увеличьте max_gen_tokens",
                max_gen_tokens, drafts
            )
        })?,
    };
    let grammar = build_signal_envelope_grammar_with_think_budget(&contract, Some(think_budget_chars));

    // Промпт собирается ровно тем же путём, что и у слоёв воронки: тот же
    // `build_system_prompt` (INCLUDE, лимит ответа, протокол честности).
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

    Ok(FlatRuntime {
        contract,
        grammar,
        system_prompt,
        think_budget_chars,
        disable_reasoning: agent.disable_reasoning,
        elements: contract_elements,
    })
}

/// Контракт сигнала плоского агента — из боевого `signals/root.schema.json`.
fn flat_contract(agents_dir: &Path) -> Result<SignalContract, String> {
    load_all_signal_contracts(&agents_dir.join("psychotherapist").join("signals"))
        .get(FLAT_AGENT)
        .cloned()
        .ok_or_else(|| format!("нет контракта сигнала у '{}'", FLAT_AGENT))
}

/// Обязательные булевы ключи контракта в порядке объявления `required`.
fn required_bool_keys(contract: &SignalContract) -> Result<Vec<String>, String> {
    let required = contract
        .value_schema
        .get("required")
        .and_then(Value::as_array)
        .ok_or_else(|| "у контракта нет списка required".to_string())?;
    let mut keys = Vec::with_capacity(required.len());
    for name in required {
        let name = name
            .as_str()
            .ok_or_else(|| format!("в required не строка: {}", name))?;
        let is_bool = contract
            .value_schema
            .get("properties")
            .and_then(|properties| properties.get(name))
            .and_then(|property| property.get("type"))
            .and_then(Value::as_str)
            == Some("boolean");
        if !is_bool {
            return Err(format!("поле '{}' контракта не boolean", name));
        }
        keys.push(name.to_string());
    }
    Ok(keys)
}

struct FlatHarness {
    engine: LlmEngine,
    sampling_presets: SamplingPresets,
    model_params: ModelParams,
    max_gen_tokens: usize,
    agents_dir: PathBuf,
    mcp_servers_dir: PathBuf,
    bins_dir: PathBuf,
    workspace_root: PathBuf,
    gate: MentalGate,
    /// Все девять элементов — из контракта, чтобы набор вердиктов шорт-циркута не
    /// расходился с тем, что стенд проверяет.
    all_elements: Vec<String>,
}

impl FlatHarness {
    fn start(model_path: &str) -> Result<Self, String> {
        let root = probe_cases::project_root();
        let agents_dir = root.join("agents");
        let app_config = load_config_early();
        // Бюджет генерации — единственный источник границы черновика. Ноль в
        // конфиге пользователя означал бы, что границу вывести нечем и молча
        // мерить нечего: ошибка обязана быть громкой (core rules §2.2).
        let max_gen_tokens = app_config.max_gen_tokens as usize;
        if max_gen_tokens == 0 {
            return Err("в конфиге пользователя max_gen_tokens = 0".to_string());
        }

        let model_label = Path::new(model_path)
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_string())
            .unwrap_or_else(|| model_path.to_string());
        let gate = MentalGate::load(&model_label)?;
        // Набор вердиктов шорт-циркута берётся из контракта плоского агента, а не
        // из списка в коде: иначе маршрутизованный кейс публиковал бы девять
        // полей, которых бой не знает.
        let contract = load_all_signal_contracts(&agents_dir.join("psychotherapist").join("signals"))
            .get(FLAT_AGENT)
            .cloned()
            .ok_or_else(|| format!("нет контракта сигнала у '{}'", FLAT_AGENT))?;
        let all_elements = required_bool_keys(&contract)?;

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

    /// Один кейс: гейт решает маршрут, иначе — один вызов плоского валидатора.
    fn run_case(&mut self, case: &ProbeCase, runtime: &FlatRuntime) -> Result<FlatRun, CallFailure> {
        let mental = self
            .gate
            .decision(case)
            .map_err(|error| CallFailure::new(error, Vec::new()))?
            .to_string();
        if mental == mental_gate::MENTAL_NONE {
            // Воронка здесь же публикует вердикты гейта как честный `false` с
            // меткой `route: short_circuit`. Плоская форма обязана быть посчитана
            // в тех же условиях, иначе соматика у неё не измерялась бы вовсе.
            let verdicts = self
                .all_elements
                .iter()
                .map(|element| (element.clone(), false))
                .collect();
            return Ok(FlatRun {
                verdicts,
                step: Value::Null,
                route: "short_circuit",
                mental_presence: mental,
            });
        }

        // Хвост промпта — общая `finalize_system_prompt`, как у боевой ноды.
        let prompt = finalize_system_prompt(&runtime.system_prompt, FLAT_AGENT, "");
        let started = std::time::Instant::now();
        let mut step = json!({
            "layer": FLAT_AGENT,
            "elements": runtime.elements,
        });
        let (text, reasoning) = self
            .generate(case, &prompt, &runtime.grammar, runtime.disable_reasoning)
            .map_err(|error| {
                step["ms"] = json!(started.elapsed().as_millis());
                CallFailure::new(format!("валидатор не ответил: {}", error), vec![step.clone()])
            })?;
        step["chars"] = json!(text.chars().count());
        step["reasoning_chars"] = json!(reasoning.chars().count());
        step["raw"] = json!(text);
        step["ms"] = json!(started.elapsed().as_millis());

        match read_verdicts(&runtime.contract, &text, &reasoning) {
            Ok(signal_value) => {
                step["signal"] = json!(signal_value);
                Ok(FlatRun {
                    verdicts: signal_value,
                    step,
                    route: "flat",
                    mental_presence: mental,
                })
            }
            Err(error) => {
                step["error"] = json!(error);
                Err(CallFailure::new(
                    format!("валидатор не ответил: {}", error),
                    vec![step],
                ))
            }
        }
    }

    fn generate(
        &mut self,
        case: &ProbeCase,
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
            session_id: format!("flat_validator_{}", case.dataset),
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
                "probe:flat_validator",
                Some(grammar.to_string()),
                disable_reasoning,
            )
            .map_err(|error| format!("LLM не отработала: {}", error))
    }
}

struct FlatRun {
    verdicts: BTreeMap<String, bool>,
    step: Value,
    route: &'static str,
    mental_presence: String,
}

#[test]
#[ignore]
fn flat_validator_probe() {
    let model_path = std::env::var("TEST_MODEL_PATH")
        .expect("Set TEST_MODEL_PATH to a GGUF file path");
    let dataset_filter = std::env::var("PROBE_DATASET").unwrap_or_default();

    let all_cases = probe_cases::load_all_cases().expect("кейсы прочитаны");
    assert!(!all_cases.is_empty(), "ни одного кейса — замер безвреден");

    let model_label = Path::new(&model_path)
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_string())
        .unwrap_or_else(|| model_path.clone());

    let mut harness = FlatHarness::start(&model_path).expect("стенд не поднялся");
    let root = probe_cases::project_root();
    let book = AcceptanceBook::load(&root).expect("книга приёмки прочитана");

    // Границы ячеек считаются здесь, а не хранятся константами: F1 выводится из
    // `max_gen_tokens` пользователя, F2 — из боевой конфигурации слоёв воронки.
    // Обе величины обязаны меняться вместе с тем, что они описывают.
    let agents_dir = root.join("agents");
    let flat_elements = required_bool_keys(
        &flat_contract(&agents_dir).expect("контракт плоского агента читается"),
    )
    .expect("контракт обязан содержать булевы поля");
    let drafts = flat_elements.len();
    let matched_funnel_chars = funnel_total_draft_chars(&agents_dir, harness.max_gen_tokens)
        .expect("бюджет воронки считается из боевой конфигурации слоёв");
    let cells: Vec<(String, Option<usize>)> = vec![
        ("F1".to_string(), None),
        (
            "F2".to_string(),
            Some(matched_funnel_chars / drafts.max(1)),
        ),
    ];
    println!(
        "\n########## плоский валидатор: {} черновиков на вызов, граница F1 = {:?} симв., \
         граница F2 = {:?} симв. (выровнено с воронкой: {} симв. суммарно), \
         размышление выключено ##########",
        drafts,
        per_draft_budget_chars(harness.max_gen_tokens, drafts),
        matched_funnel_chars / drafts.max(1),
        matched_funnel_chars,
    );

    let cases: Vec<&ProbeCase> = all_cases
        .iter()
        .filter(|case| dataset_filter.is_empty() || dataset_filter == case.dataset)
        .collect();
    if cases.is_empty() {
        panic!("PROBE_DATASET='{}' не совпал ни с одним набором", dataset_filter);
    }

    for (cell_name, budget_override) in &cells {
        let runtime = prepare_flat(&agents_dir, harness.max_gen_tokens, *budget_override)
            .unwrap_or_else(|error| panic!("ячейка {}: {}", cell_name, error));
        let config =
            ReportConfig::of(cell_name, &runtime, harness.gate.source(), harness.max_gen_tokens);
        println!(
            "\n########## плоский валидатор, ячейка {}: граница черновика {} симв. \
             на {} черновиков, размышление выключено: {} ##########",
            cell_name,
            runtime.think_budget_chars,
            runtime.elements.len(),
            runtime.disable_reasoning
        );

        let report_path = probe_cases::results_dir()
            .join(format!("flat_validator_{}_{}.json", cell_name, model_label));
        let mut rows = config
            .read_rows(&report_path)
            .unwrap_or_else(|error| panic!("{}", error));

        for (index, case) in cases.iter().enumerate() {
            let started = std::time::Instant::now();
            let mut row = json!({
                "dataset": case.dataset,
                "id": case.id,
                "text": case.text,
                "expected": case.expected.clone().unwrap_or_default(),
                "has_labels": case.expected.is_some(),
            });

            match harness.run_case(case, &runtime) {
                Ok(run) => {
                    row["steps"] = json!([run.step]);                    row["unparsed_calls"] = json!(0);
                    row["route"] = json!(run.route);
                    row["mental_presence"] = json!(run.mental_presence);
                    match score_fields(case.expected.as_ref(), &run.verdicts) {
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
                    row["unparsed_calls"] = json!(1);
                    row["error"] = json!(failure.message);
                    // Сырой ответ обязателен при ЛЮБОМ исходе: без него отказ
                    // неотличим от «модель не дошла до конверта».
                    row["steps"] = json!(failure.steps);
                    if let Some(expected) = &case.expected {
                        let (correct, fields, tp, tn, fp, fn_) = score_failed_case(expected);
                        row["correct"] = json!(correct);
                        row["fields"] = json!(fields);
                        row["tp"] = json!(tp);
                        row["tn"] = json!(tn);
                        row["fp"] = json!(fp);
                        row["fn"] = json!(fn_);
                    }
                    eprintln!(
                        "[probe] ПРОВАЛ [{}] {} {}: {}",
                        cell_name, case.dataset, case.id, failure.message
                    );
                }
            }

            upsert_row(&mut rows, row);
            // Отчёт после каждого кейса: длинный прогон нельзя терять целиком.
            write_report(&report_path, &model_path, &rows, &config, &book);
        }

        let summary = summarize(&rows);
        let gates = evaluate_gates(&book, &summary, &rows);
        write_report(&report_path, &model_path, &rows, &config, &book);
        println!(
            "\n=== плоский валидатор, ячейка {} ({}) ===\n{}\nГейты:\n{}\nОтчёт: {}",
            cell_name,
            model_label,
            serde_json::to_string_pretty(&summary).expect("json"),
            serde_json::to_string_pretty(&gates).expect("json"),
            report_path.display()
        );
    }
}

fn write_report(
    path: &Path,
    model_path: &str,
    rows: &[Value],
    config: &ReportConfig,
    book: &AcceptanceBook,
) {
    let summary = summarize(rows);
    let report = json!({
        "note": "Плоская форма проверки 9 элементов невроза: ОДИН вызов агента \
                 backend/validator_flat.md, девять черновиков по числу обязательных \
                 полей контракта, грамматика — signals::build_signal_envelope_grammar_with_think_budget \
                 с границей signals::per_draft_budget_chars. Контракт — signals/root.schema.json, \
                 критерии — element_validation_rules.yaml (те же, что у слоёв воронки). \
                 Маршрут кейса — из боевого гейта mental_presence (артефакт fact_extractor_mental_*.json), \
                 золото для маршрутизации не используется. Ячейки F1 (граница из max_gen_tokens) \
                 и F2 (бюджет выровнен с воронкой) отличаются ТОЛЬКО границей черновика. \
                 Гейты G0-G5 объявлены ДО прогона в tasks/11.10.26 Плоский валидатор против \
                 трёхслойной воронки.md §5. Правки критериев in-sample, поэтому числа этого \
                 прогона — измерение, но НЕ доказательство обобщающей способности \
                 (AGENTS.md §00.3.1); промпты плоской формы и воронки не идентичны \
                 (там же L1).",
        "model": model_path,
        "system": book.system_label(ACCEPTANCE_SYSTEM).unwrap_or(ACCEPTANCE_SYSTEM),
        "agent": config.agent,
        "cell": config.cell,
        "drafts": config.drafts,
        "think_budget_chars": config.think_budget_chars,
        "max_gen_tokens": config.max_gen_tokens,
        "disable_reasoning": config.disable_reasoning,
        "gate_source": config.gate_source,
        "routing": routing_stats(rows, "flat"),
        "summary": summary,
        "gates": evaluate_gates(book, &summary, rows),
        "rows": rows,
    });
    super::probe_call::write_report(path, &report);
}

/// Сколько кейсов пошло мимо валидатора по решению боевого гейта — общий счёт
/// живёт в `probe_report::routing_stats`, здесь только имя маршрута.

/// Конфигурация замера; она же хранится в отчёте.
///
/// Смешивание строк от разных конфигураций в одном отчёте — ложь в цифрах. При
/// расхождении прогон останавливается, а не дописывает кейсы в чужой отчёт.
struct ReportConfig {
    cell: String,
    agent: &'static str,
    drafts: usize,
    think_budget_chars: usize,
    disable_reasoning: bool,
    gate_source: String,
    max_gen_tokens: usize,
}

impl ReportConfig {
    fn of(cell: &str, runtime: &FlatRuntime, gate_source: &Path, max_gen_tokens: usize) -> Self {
        Self {
            cell: cell.to_string(),
            agent: FLAT_AGENT,
            drafts: runtime.elements.len(),
            think_budget_chars: runtime.think_budget_chars,
            disable_reasoning: runtime.disable_reasoning,
            gate_source: gate_source.display().to_string(),
            max_gen_tokens,
        }
    }

    fn as_json(&self) -> Value {
        json!({
            "cell": self.cell,
            "agent": self.agent,
            "drafts": self.drafts,
            "think_budget_chars": self.think_budget_chars,
            "disable_reasoning": self.disable_reasoning,
            "gate_source": self.gate_source,
            "max_gen_tokens": self.max_gen_tokens,
        })
    }

    fn read_rows(&self, path: &Path) -> Result<Vec<Value>, String> {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Ok(Vec::new());
        };
        let report: Value = serde_json::from_str(&text)
            .map_err(|error| format!("{} не разобран: {}", path.display(), error))?;
        let previous = json!({
            "cell": report.get("cell").cloned().unwrap_or(Value::Null),
            "agent": report.get("agent").cloned().unwrap_or(Value::Null),
            "drafts": report.get("drafts").cloned().unwrap_or(Value::Null),
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

/// Гейты по книге приёмки — единственному источнику порогов
/// (`test_cases/new_tests_for_validator/acceptance_gates.json`).
///
/// Считает и «сколько полей набрала система»: без этого числа гейт «выиграла ли
/// система у базы» не измеряем.
fn evaluate_gates(book: &AcceptanceBook, summary: &Value, rows: &[Value]) -> Value {
    let unparsed: usize = rows
        .iter()
        .filter_map(|row| row.get("unparsed_calls").and_then(Value::as_u64))
        .map(|count| count as usize)
        .sum();
    let own_correct: i64 = rows
        .iter()
        .filter_map(|row| row.get("correct").and_then(Value::as_i64))
        .sum();
    book.evaluate(ACCEPTANCE_SYSTEM, summary, own_correct, unparsed)
        .unwrap_or_else(|error| {
            // Падение здесь означает, что стенд запущен без приёмки. Молчаливый
            // пустой результат выглядел бы как «гейтов нет, значит всё прошло».
            panic!("приёмка не посчитана: {}", error)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agents_dir() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("корень проекта")
            .join("agents")
    }

/// Порядок черновиков обязан совпадать с `required` контракта: промпт велит писать
/// элементы подряд в этом порядке, грамматика ждёт ровно его. Расхождение сделало бы
/// задание модели невыполнимым, и стенд молча замерял бы отказ.
///
/// **Список элементов берётся из контракта, а не из кода стенда**: иначе стенд
/// содержал бы вторую копию состава элементов и расхождение с контрактом осталось бы
/// незамеченным.
#[test]
fn prompt_draft_order_matches_contract() {
    let runtime = prepare_flat(&agents_dir(), 2048, None).expect("плоский слой собран");
    assert!(!runtime.elements.is_empty(), "контракт без полей — замер бессмыслен");
    let prompt = runtime.system_prompt.clone();
    let mut cursor = 0usize;
    for element in &runtime.elements {
        let at = prompt[cursor..]
            .find(element.as_str())
            .unwrap_or_else(|| panic!("в промпте нет {} после позиции {}", element, cursor));
        cursor += at + element.len();
    }
    // Число черновиков обязано совпадать с числом обязательных полей.
    assert_eq!(
        runtime.elements.len(),
        required_field_count(&runtime.contract.value_schema),
        "в промпте один черновик на элемент, но полей в контракте другое число"
    );
}

    /// Плоская форма и слои воронки обязаны читать один файл критериев. Иначе
    /// сравниваются не две архитектуры, а два разных определения элемента.
    #[test]
    fn flat_and_funnel_share_the_criteria_file() {
        let root = agents_dir();
        let team = root.join("psychotherapist");
        let agents = load_agents(&root).expect("агенты прочитаны");
        let include = "<<INCLUDE: ../database/element_validation_rules.yaml>>";
        for id in ["validator_flat", "validator_surface", "validator_facade", "validator_core"] {
            let agent = agents
                .iter()
                .find(|agent| agent.id == id)
                .unwrap_or_else(|| panic!("агент {} не найден", id));
            assert!(
                agent.system_prompt.contains(include),
                "агент {} не подключает боевую базу критериев",
                id
            );
        }
        assert!(
            team.join("database/element_validation_rules.yaml").exists(),
            "файл критериев воронки не найден"
        );
    }

    /// Границы ячеек обязаны различаться, и F2 обязана быть **выведена из боевой
    /// конфигурации воронки**, а не задана числом в коде стенда.
    ///
    /// Числа 398 и 1195 — результат, а не ожидание: тест проверяет свойства
    /// (F2 выровнена с воронкой, границы различны), а не повторяет их.
    #[test]
    fn cells_have_distinct_boundaries_and_f2_matches_the_funnel() {
        let dir = agents_dir();
        let runtime_f1 = prepare_flat(&dir, 2048, None).expect("F1");
        let drafts = runtime_f1.elements.len();
        let matched = funnel_total_draft_chars(&dir, 2048).expect("бюджет воронки посчитан");
        let runtime_f2 = prepare_flat(&dir, 2048, Some(matched / drafts)).expect("F2");

        assert_ne!(
            runtime_f1.think_budget_chars, runtime_f2.think_budget_chars,
            "одинаковые границы превратили бы две ячейки в один и тот же замер"
        );
        // F1 обязан совпасть с границей из `max_gen_tokens` — иначе ячейка «как есть»
        // перестала бы описывать бой.
        assert_eq!(
            Some(runtime_f1.think_budget_chars),
            per_draft_budget_chars(2048, drafts),
            "F1 обязана быть границей из max_gen_tokens"
        );
        // F2 обязана дать плоской форме столько же места, сколько воронка. Сравнение —
        // с точностью до одного черновика: бюджет делится на целое число черновиков,
        // и при некратном сумме расхождение в остатке неустранимо. Требовать
        // точное равенство значило бы требовать невыполнимое.
        let aligned = runtime_f2.think_budget_chars * drafts;
        assert!(
            aligned.abs_diff(matched) <= drafts,
            "F2 дала {} симв. размышления против {} у воронки — расхождение больше одного черновика",
            aligned,
            matched
        );
        // Воронка обязана состоять более чем из одного слоя: выравнивание суммы по
        // единственному слою — это выравнивание с самим собой, а не контроль.
        let layers = validator_layers::load(&dir.join("psychotherapist")).expect("слои прочитаны");
        assert!(
            layers.len() > 1,
            "воронка объявлена одним слоем — F2 теряет смысл"
        );
    }

    /// Граница обязана попасть в грамматику, а не остаться в конфигурации стенда:
    /// иначе стенд меряет не то, что объявляет в отчёте.
    #[test]
    fn boundary_reaches_the_grammar() {
        let dir = agents_dir();
        let drafts = prepare_flat(&dir, 2048, None).expect("слой собран").elements.len();
        let matched = funnel_total_draft_chars(&dir, 2048).expect("бюджет воронки посчитан");
        for budget in [
            per_draft_budget_chars(2048, drafts).expect("граница F1"),
            matched / drafts,
        ] {
            let runtime =
                prepare_flat(&dir, 2048, Some(budget)).expect("слой собран");
            assert!(
                runtime.grammar.contains(&format!("{{0,{}}}", budget)),
                "в грамматике нет границы {{0,{}}} — граница не дошла до движка: {}",
                budget,
                runtime.grammar.lines().next().unwrap_or_default()
            );
        }
    }

    /// Книга приёмки обязана существовать, читаться и содержать приёмку плоской
    /// формы. Пустая или подменённая книга сделала бы прогон без вердикта.
    #[test]
    fn acceptance_book_declares_the_flat_form() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("корень проекта");
        let book = AcceptanceBook::load(root).expect("книга приёмки читается");
        let ids = book.gate_ids(ACCEPTANCE_SYSTEM).expect("приёмка плоской формы есть");
        assert!(
            !ids.is_empty(),
            "у системы '{} нет ни одного гейта — прогон без приёмки",
            ACCEPTANCE_SYSTEM
        );
        let acceptance = book.system(ACCEPTANCE_SYSTEM).expect("приёмка");
        assert_eq!(acceptance.max_unparsed_calls, 0, "нераспарсенный вызов — отказ");
        // База для сравнения обязана существовать: гейт «лучше ли» без базы — пустая
        // строка, а не утверждение.
        let gain = acceptance.gain.as_ref().expect("гейт выигрыша объявлен");
        let reference = book.reference(&gain.reference).expect("база есть");
        assert!(reference.correct_fields > 0, "у базы нет измеренных полей");
    }

    /// Гейты берутся из книги приёмки, а не из кода стенда: проверка, что книга
    /// читается и содержит нужную систему, уже сделана выше. Здесь — что стенд
    /// действительно считает по книге, а не по своей копии порогов.
    #[test]
    fn gates_come_from_the_book_not_from_code() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("корень проекта");
        let book = AcceptanceBook::load(root).expect("книга приёмки читается");
        let summary = json!({"overall": {"accuracy": null}});
        let gates = evaluate_gates(&book, &summary, &[]);
        let ids = book.gate_ids(ACCEPTANCE_SYSTEM).expect("гейты есть");
        for id in ids {
            assert!(
                gates.get(&id).is_some(),
                "гейт '{} не посчитан стендом — книга и стенд разошлись",
                id
            );
        }
    }

    /// Смешение конфигураций в одном отчёте запрещено: после смены границы старые
    /// строки относятся к другой системе.
    #[test]
    fn report_config_rejects_foreign_configuration() {
        let dir = std::env::temp_dir().join("flat_validator_probe_config_test");
        std::fs::create_dir_all(&dir).expect("временная папка создана");
        let path = dir.join("report.json");
        std::fs::write(
            &path,
            json!({
                "cell": "F1",
                "agent": FLAT_AGENT,
                "drafts": 9,
                "think_budget_chars": 398,
                "disable_reasoning": true,
                "gate_source": "gate.json",
                "max_gen_tokens": 2048,
                "rows": [],
            })
            .to_string(),
        )
        .expect("отчёт записан");

        let runtime = prepare_flat(&agents_dir(), 2048, None).expect("слой собран");
        let matching = ReportConfig::of("F1", &runtime, Path::new("gate.json"), 2048);
        matching.read_rows(&path).expect("та же конфигурация читается");

        let other = ReportConfig::of("F2", &runtime, Path::new("gate.json"), 2048);
        let error = other
            .read_rows(&path)
            .expect_err("чужая конфигурация обязана отвергаться");
        assert!(error.contains("другой конфигурацией"), "{}", error);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Конверт принимается только как `emit_signal` с полным набором полей —
    /// тем же путём, что и бой. Частичный или чужой конверт должен быть отказом.
    #[test]
    fn envelope_is_parsed_through_the_battle_path() {
        let runtime = prepare_flat(&agents_dir(), 2048, None).expect("слой соббран");
        let full = format!(
            "{{\"tool\":\"emit_signal\",\"arguments\":{{\"key\":\"{}\",\"value\":{{{}}}}}}}",
            runtime.contract.key,
            runtime
                .elements
                .iter()
                .map(|e| format!("\"{}\":false", e))
                .collect::<Vec<_>>()
                .join(",")
        );
        let verdicts = read_verdicts(&runtime.contract, &full, "").expect("полный конверт разобран");
        assert_eq!(verdicts.len(), runtime.elements.len());

        let partial = format!(
            "{{\"tool\":\"emit_signal\",\"arguments\":{{\"key\":\"{}\",\"value\":{{\"e1\":false}}}}}}",
            runtime.contract.key
        );
        assert!(
            read_verdicts(&runtime.contract, &partial, "").is_err(),
            "конверт без всех полей обязан быть отказом, а не частичным вердиктом"
        );

        let wrong_tool = format!(
            "{{\"tool\":\"batch_get_agent_report\",\"arguments\":{{\"key\":\"{}\",\"value\":{{}}}}}}",
            runtime.contract.key
        );
        let error = read_verdicts(&runtime.contract, &wrong_tool, "").expect_err("чужой инструмент");
        assert!(error.contains("emit_signal"), "{}", error);

        // Конверт в канале reasoning — тоже валидный ответ.
        assert!(
            read_verdicts(&runtime.contract, "", &full).is_ok(),
            "конверт из reasoning_content обяжен приниматься"
        );
    }
}