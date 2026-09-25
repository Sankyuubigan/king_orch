use std::path::Path;

use super::run_pipeline_test_cli;

#[test]
#[ignore]
fn test_psychotherapist_validator_e1_e9_e2e() {
    let project_root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let model_path = std::env::var("TEST_MODEL_PATH")
        .expect("Для теста задайте TEST_MODEL_PATH");

    let result = run_pipeline_test_cli(
        "psychotherapist_validator_e1_e9",
        &model_path,
        project_root,
    )
    .expect("pipeline test упал");

    assert!(
        result.overall_passed,
        "pipeline test НЕ пройден: {:?}",
        result.level1_structure.details
    );
}
