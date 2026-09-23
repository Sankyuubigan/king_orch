# Каталог конфигурации King Orch (сгенерировано автоматически)

> ⚠️ ФАЙЛ СГЕНЕРИРОВАН скриптом `scripts/gen_docs.cjs`. **НЕ ПРАВИТЬ РУКАМИ** —
> правки будут перезаписаны при следующей генерации. Правь код, затем перегенерируй.

## Структуры конфигурации (infra/config.rs)

### AppConfig

- `tabs`: `Vec<TabState>`
- `active_tab`: `Option<String>`
- `models`: `Vec<String>`
- `last_model`: `Option<String>`
- `last_agent`: `Option<String>`
- `models_dir`: `Option<String>`
- `model_params`: `HashMap<String`
- `context_size`: `u32`
- `max_gen_tokens`: `u32`
- `reasoning_budget`: `u32`
- `kv_quant_keys`: `bool`
- `kv_quant_values`: `bool`
- `theme`: `String`
- `prompt_format`: `String`
- `confidence_threshold`: `f32`
- `show_advanced_features`: `bool`
- `show_folder_agents`: `bool`
- `mmproj_files`: `HashMap<String`
- `model_meta`: `HashMap<String`
- `llamacpp_dir`: `Option<String>`
- `engine_source`: `Option<String>`
- `engine_variant`: `Option<String>`
- `allow_error_reports`: `bool`
- `chat_font_scale`: `f32`
- `translator_model`: `Option<String>`
- `translator_lang`: `String`
- `workdir`: `Option<String>`
- `two_phase_default`: `bool`

