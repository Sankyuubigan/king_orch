//! 🚪 Модуль coding-бенчмарка LLM — публичный контракт.
//! API-слой обращается к нему ТОЛЬКО через этот фасад.

pub mod evaluator;
pub mod kv_probe;
pub mod report;
pub mod runner;
pub mod tasks;

pub use evaluator::{assemble_solution, run_command, strip_markdown_fences, ExecVerdict};
pub use kv_probe::{probe_max_ctx_f16, KvProbeResult};
pub use report::{write_artifacts, write_report, ModelRunSummary, ReportSummary, TaskResultRecord};
pub use runner::{run_coding_bench, CodingBenchOptions, ModelToRun};
pub use tasks::{list_suites, load_suite_tasks, CodingTask, SuiteInfo, TaskFile};