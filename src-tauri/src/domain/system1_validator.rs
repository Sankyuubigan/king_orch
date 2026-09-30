use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;
use ort::value::Tensor;
use serde::{Deserialize, Serialize};
use tokenizers::Tokenizer;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationRule {
    pub name: String,
    pub question_type: Option<String>,
    pub question: String,
    pub true_criteria: String,
    pub false_criteria: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationElementResult {
    pub verdict: bool,
    pub p_true: f32,
    pub answer_confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct System1ValidationReport {
    pub e1: bool,
    pub e2: bool,
    pub e3: bool,
    pub e4: bool,
    pub e5: bool,
    pub e6: bool,
    pub e7: bool,
    pub e8: bool,
    pub e9: bool,
    pub elapsed_ms: u64,
    pub elements: BTreeMap<String, ValidationElementResult>,
}

pub struct LayaValidator {
    session: Mutex<Session>,
    tokenizer: Tokenizer,
    cls_id: u32,
    sep_id: u32,
    mask_id: u32,
    pad_id: u32,
    mask_token_str: String,
}

impl LayaValidator {
    pub fn load(model_dir: &Path) -> Result<Self, String> {
        let model_path = if model_dir.is_file() {
            model_dir.to_path_buf()
        } else {
            model_dir.join("model.onnx")
        };

        if !model_path.exists() {
            return Err(format!("ONNX model not found at {}", model_path.display()));
        }

        let tokenizer_path = if model_dir.is_file() {
            model_dir
                .parent()
                .unwrap_or(Path::new("."))
                .join("tokenizer")
                .join("tokenizer.json")
        } else if model_dir.join("tokenizer").join("tokenizer.json").exists() {
            model_dir.join("tokenizer").join("tokenizer.json")
        } else {
            model_dir.join("tokenizer.json")
        };

        if !tokenizer_path.exists() {
            return Err(format!(
                "Tokenizer not found at {}",
                tokenizer_path.display()
            ));
        }

        let tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| format!("Failed to load tokenizer from {}: {}", tokenizer_path.display(), e))?;

        let cls_id = tokenizer
            .token_to_id("<bos>")
            .or_else(|| tokenizer.token_to_id("[CLS]"))
            .unwrap_or(2);
        let sep_id = tokenizer
            .token_to_id("<eos>")
            .or_else(|| tokenizer.token_to_id("[SEP]"))
            .unwrap_or(1);
        let mask_id = tokenizer
            .token_to_id("<mask_token>")
            .or_else(|| tokenizer.token_to_id("<mask_1>"))
            .or_else(|| tokenizer.token_to_id("<mask>"))
            .or_else(|| tokenizer.token_to_id("[MASK]"))
            .unwrap_or(4);
        let pad_id = tokenizer
            .token_to_id("<pad>")
            .or_else(|| tokenizer.token_to_id("[PAD]"))
            .unwrap_or(0);

        let mask_token_str = tokenizer
            .id_to_token(mask_id)
            .unwrap_or_else(|| "<mask_token>".to_string());

        let session = Session::builder()
            .map_err(|e| format!("ort builder error: {}", e))?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|e| format!("ort opt level error: {}", e))?
            .with_intra_threads(4)
            .map_err(|e| format!("ort threads error: {}", e))?
            .commit_from_file(&model_path)
            .map_err(|e| format!("ort commit error for {}: {}", model_path.display(), e))?;

        Ok(Self {
            session: Mutex::new(session),
            tokenizer,
            cls_id,
            sep_id,
            mask_id,
            pad_id,
            mask_token_str,
        })
    }

    pub fn load_rules(rules_path: &Path) -> Result<Vec<(usize, ValidationRule)>, String> {
        let content = std::fs::read_to_string(rules_path)
            .map_err(|e| format!("Failed to read rules from {}: {}", rules_path.display(), e))?;

        let doc: serde_yaml::Value = serde_yaml::from_str(&content)
            .map_err(|e| format!("Failed to parse rules YAML from {}: {}", rules_path.display(), e))?;

        let elements = doc
            .get("elements")
            .and_then(|e| e.as_mapping())
            .ok_or_else(|| "Missing 'elements' map in rules file".to_string())?;

        let mut list = Vec::new();
        for num in 1..=9 {
            let key = serde_yaml::Value::Number(num.into());
            if let Some(el_val) = elements.get(&key) {
                let rule: ValidationRule = serde_yaml::from_value(el_val.clone())
                    .map_err(|e| format!("Failed to parse element {}: {}", num, e))?;
                list.push((num, rule));
            }
        }

        if list.len() != 9 {
            return Err(format!("Expected 9 elements in rules, found {}", list.len()));
        }

        Ok(list)
    }

    pub fn validate(
        &self,
        text: &str,
        rules: &[(usize, ValidationRule)],
    ) -> Result<System1ValidationReport, String> {
        let start = Instant::now();

        let run_a = self.infer_batch(text, rules, ("A", "B"))?;
        let run_b = self.infer_batch(text, rules, ("B", "A"))?;

        let mut elements_map = BTreeMap::new();
        let mut verdicts = [false; 9];

        for (i, (num, _rule)) in rules.iter().enumerate() {
            let key = format!("e{}", num);
            let p_a = run_a[i];
            let p_b = run_b[i];
            let p_true = (p_a + p_b) / 2.0;
            let verdict = p_true >= 0.5;
            let ans_conf = (p_true.max(1.0 - p_true)).min(1.0).max(0.0);

            verdicts[i] = verdict;
            elements_map.insert(
                key,
                ValidationElementResult {
                    verdict,
                    p_true,
                    answer_confidence: ans_conf,
                },
            );
        }

        let elapsed_ms = start.elapsed().as_millis() as u64;

        Ok(System1ValidationReport {
            e1: verdicts[0],
            e2: verdicts[1],
            e3: verdicts[2],
            e4: verdicts[3],
            e5: verdicts[4],
            e6: verdicts[5],
            e7: verdicts[6],
            e8: verdicts[7],
            e9: verdicts[8],
            elapsed_ms,
            elements: elements_map,
        })
    }

    fn infer_batch(
        &self,
        state: &str,
        rules: &[(usize, ValidationRule)],
        labels: (&str, &str),
    ) -> Result<Vec<f32>, String> {
        const MAX_LEN: usize = 2048;
        const HEAD_MAX_LEN: usize = 512;
        const OPTION_TOKEN_LIMIT: usize = 48;

        let n = rules.len();
        let mut items_ids: Vec<Vec<u32>> = Vec::with_capacity(n);
        let mut items_markers: Vec<Vec<usize>> = Vec::with_capacity(n);

        let sanitized_state = state.replace(&self.mask_token_str, " ");
        let state_encoding = self
            .tokenizer
            .encode(sanitized_state.as_str(), false)
            .map_err(|e| format!("Failed to tokenize state: {}", e))?;
        let state_ids: Vec<u32> = state_encoding.get_ids().to_vec();

        for (_num, rule) in rules {
            let ins = rule.question.replace(&self.mask_token_str, " ");
            let head_text = format!("noul question: {}", ins);
            let head_enc = self
                .tokenizer
                .encode(head_text.as_str(), false)
                .map_err(|e| format!("Failed to tokenize head: {}", e))?;
            let mut head_ids: Vec<u32> = head_enc.get_ids().to_vec();

            let false_text = if rule.false_criteria.trim().is_empty() {
                format!("{}: no, the statement does not hold", labels.1)
            } else {
                format!("{}: {}", labels.1, rule.false_criteria.trim())
            };
            let true_text = if rule.true_criteria.trim().is_empty() {
                format!("{}: yes, the statement holds", labels.0)
            } else {
                format!("{}: {}", labels.0, rule.true_criteria.trim())
            };

            let mut opt_ids: Vec<Vec<u32>> = Vec::new();
            for opt in &[false_text, true_text] {
                let opt_str = format!(" {}", opt.replace(&self.mask_token_str, " "));
                let enc = self
                    .tokenizer
                    .encode(opt_str.as_str(), false)
                    .map_err(|e| format!("Failed to tokenize option: {}", e))?;
                let mut opt_tokens: Vec<u32> = enc.get_ids().to_vec();
                if opt_tokens.len() > OPTION_TOKEN_LIMIT {
                    opt_tokens.truncate(OPTION_TOKEN_LIMIT);
                }
                let mut full_opt = vec![self.mask_id];
                full_opt.extend(opt_tokens);
                opt_ids.push(full_opt);
            }

            let opt_sum: usize = opt_ids.iter().map(|o| o.len()).sum();
            let mut opt_budget = if HEAD_MAX_LEN > opt_sum {
                HEAD_MAX_LEN - opt_sum
            } else {
                0
            };
            if opt_budget < 16 {
                let per = (HEAD_MAX_LEN.saturating_sub(16) / opt_ids.len().max(1)).max(4);
                for o in &mut opt_ids {
                    if o.len() > per {
                        o.truncate(per);
                    }
                }
                let opt_sum_new: usize = opt_ids.iter().map(|o| o.len()).sum();
                opt_budget = if HEAD_MAX_LEN > opt_sum_new {
                    HEAD_MAX_LEN - opt_sum_new
                } else {
                    0
                };
            }

            if head_ids.len() > opt_budget.max(8) {
                head_ids.truncate(opt_budget.max(8));
            }

            let mut ids = vec![self.cls_id];
            ids.extend(head_ids);
            ids.push(self.sep_id);

            let mut markers = Vec::new();
            for o in opt_ids {
                markers.push(ids.len());
                ids.extend(o);
            }
            ids.push(self.sep_id);

            let room = if MAX_LEN > ids.len() + 1 {
                MAX_LEN - ids.len() - 1
            } else {
                0
            };
            let st_len = state_ids.len().min(room);
            ids.extend_from_slice(&state_ids[..st_len]);
            ids.push(self.sep_id);

            if ids.len() > MAX_LEN {
                ids.truncate(MAX_LEN);
            }
            markers.retain(|&m| m < MAX_LEN);

            items_ids.push(ids);
            items_markers.push(markers);
        }

        let max_seq_len = items_ids.iter().map(|v| v.len()).max().unwrap_or(1);
        let kmax = items_markers.iter().map(|v| v.len()).max().unwrap_or(2);

        let mut input_ids = vec![self.pad_id as i64; n * max_seq_len];
        let mut attention_mask = vec![0i64; n * max_seq_len];
        let mut marker_pos = vec![0i64; n * kmax];
        let mut marker_mask = vec![false; n * kmax];
        let qtype = vec![2i64; n];

        for i in 0..n {
            let seq_len = items_ids[i].len();
            for j in 0..seq_len {
                input_ids[i * max_seq_len + j] = items_ids[i][j] as i64;
                attention_mask[i * max_seq_len + j] = 1;
            }
            let k = items_markers[i].len();
            for m in 0..k {
                marker_pos[i * kmax + m] = items_markers[i][m] as i64;
                marker_mask[i * kmax + m] = true;
            }
        }

        let in_input_ids = Tensor::from_array((vec![n, max_seq_len], input_ids))
            .map_err(|e| format!("ort tensor error (input_ids): {}", e))?;
        let in_attention_mask = Tensor::from_array((vec![n, max_seq_len], attention_mask))
            .map_err(|e| format!("ort tensor error (attention_mask): {}", e))?;
        let in_marker_pos = Tensor::from_array((vec![n, kmax], marker_pos))
            .map_err(|e| format!("ort tensor error (marker_pos): {}", e))?;
        let in_marker_mask = Tensor::from_array((vec![n, kmax], marker_mask))
            .map_err(|e| format!("ort tensor error (marker_mask): {}", e))?;
        let in_qtype = Tensor::from_array((vec![n], qtype))
            .map_err(|e| format!("ort tensor error (qtype): {}", e))?;

        let inputs = ort::inputs![
            "input_ids" => in_input_ids,
            "attention_mask" => in_attention_mask,
            "marker_pos" => in_marker_pos,
            "marker_mask" => in_marker_mask,
            "qtype" => in_qtype,
        ];

        let mut session = self
            .session
            .lock()
            .map_err(|e| format!("ort session mutex error: {}", e))?;

        let outputs = session
            .run(inputs)
            .map_err(|e| format!("ort run error: {}", e))?;

        let (_shape, logits_raw) = outputs["logits"]
            .try_extract_tensor::<f32>()
            .map_err(|e| format!("ort extract logits error: {}", e))?;

        let mut p_trues = Vec::with_capacity(n);
        for i in 0..n {
            let row_start = i * kmax;
            let z0 = logits_raw[row_start];
            let z1 = logits_raw[row_start + 1];
            let max_z = z0.max(z1);
            let e0 = (z0 - max_z).exp();
            let e1 = (z1 - max_z).exp();
            let sum = (e0 + e1).max(1e-12);
            let p_slot1 = e1 / sum;
            p_trues.push(p_slot1);
        }

        Ok(p_trues)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_laya_validator_all_tasks() {
        let model_dir = Path::new("test/laya_probe/models/laya-multilingual-onnx");
        if !model_dir.exists() {
            eprintln!("Model not downloaded at test/laya_probe/models/laya-multilingual-onnx, skipping test");
            return;
        }

        let validator = LayaValidator::load(model_dir).expect("load validator");
        let rules_path = Path::new("test_cases/new_tests_for_validator/element_validation_rules_prod_noul.yaml");
        let rules = LayaValidator::load_rules(rules_path).expect("load rules");

        // Test task1
        let task1_path = Path::new("test_cases/fixtures/psychotherapist_validator_e1_e9/task1.md");
        let task1_text = std::fs::read_to_string(task1_path).expect("read task1");
        let rep1 = validator.validate(&task1_text, &rules).expect("validate task1");

        println!(
            "Task1 validation report: e1={}, e2={}, e3={}, e4={}, e5={}, e6={}, e7={}, e8={}, e9={} (elapsed: {}ms)",
            rep1.e1, rep1.e2, rep1.e3, rep1.e4, rep1.e5, rep1.e6, rep1.e7, rep1.e8, rep1.e9, rep1.elapsed_ms
        );

        assert!(rep1.elapsed_ms < 1000, "Inference should be under 1s, got {}ms", rep1.elapsed_ms);
        assert_eq!(rep1.e1, true);
        assert_eq!(rep1.e2, true);
        assert_eq!(rep1.e3, false);
        assert_eq!(rep1.e4, false);
        assert_eq!(rep1.e5, true);
        assert_eq!(rep1.e6, true);
        assert_eq!(rep1.e7, true);
        assert_eq!(rep1.e8, true);
        assert_eq!(rep1.e9, true);

        // Test task2
        let task2_path = Path::new("test_cases/fixtures/psychotherapist_validator_e1_e9/task2.md");
        let task2_text = std::fs::read_to_string(task2_path).expect("read task2");
        let rep2 = validator.validate(&task2_text, &rules).expect("validate task2");

        println!(
            "Task2 validation report: e1={}, e2={}, e3={}, e4={}, e5={}, e6={}, e7={}, e8={}, e9={} (elapsed: {}ms)",
            rep2.e1, rep2.e2, rep2.e3, rep2.e4, rep2.e5, rep2.e6, rep2.e7, rep2.e8, rep2.e9, rep2.elapsed_ms
        );

        assert_eq!(rep2.e1, true);
        assert_eq!(rep2.e2, false);
        assert_eq!(rep2.e3, false);
        assert_eq!(rep2.e4, false);
        assert_eq!(rep2.e5, false);
        assert_eq!(rep2.e6, false);
        assert_eq!(rep2.e7, false);
        assert_eq!(rep2.e8, false);
        assert_eq!(rep2.e9, false);
    }
}
