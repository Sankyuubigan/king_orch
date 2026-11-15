//! 🚪 Модуль coding-бенчмарка LLM — публичный контракт.
//! API-слой обращается к нему ТОЛЬКО через этот фасад.

pub mod evaluator;
pub mod kv_probe;
pub mod mcq;
pub mod report;
pub mod runner;
pub mod tasks;

pub use mcq::grade_mcq; // extract_letter/extract_number — внутренности mcq.rs (дверь не экспортирует мёртвое)
pub use runner::{run_coding_bench, CodingBenchOptions, ModelToRun};
pub use tasks::{list_suites, SuiteInfo};