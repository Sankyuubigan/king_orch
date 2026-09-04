Такие грамматики необязательно писать вручную, и для больших схем это не требуется. llama.cpp умеет конвертировать подмножество JSON Schema в GBNF — либо заранее с помощью скрипта json_schema_to_grammar.py, либо при каждом запросе, если передать схему в поле json_schema (или response_format для /chat/completions) серверу llama.cpp. То есть схему вы храните в привычном формате JSON Schema, который и так понятен всему вашему стеку, а движок сам компилирует из нее грамматику. 

> **⚠️ КРИТИЧЕСКОЕ ПРАВИЛО: Подчёркивания (`_`) запрещены в именах правил GBNF.**
> GBNF-парсер llama.cpp (`is_word_char()` в `common/grammar-parser.cpp`) допускает только:
> - `a-z`, `A-Z` (буквы)
> - `0-9` (цифры)
> - `-` (дефис)
>
> Подчёркивание `_` **НЕ является** допустимым символом. Попытка использовать его вызывает ошибку:
> ```
> error parsing grammar: expecting newline or end at _content "azaar" json_answer
> ```
> Парсер видит `thought_content` как два отдельных токена: `thought` + `_content`, где `_content` неожиданно.
>
> **Как писать:** все имена правил — через дефис: `thought-content`, `json-answer`, `answer-body`.
>
> Это касается:
> - Рукописных `.gbnf` файлов в `agents/**/grammars/`
> - Программно сгенерированных грамматик в `signals.rs` (`build_signal_envelope_grammar()`, `build_value_grammar()`)
> - Гибридных GBNF-обёрток в `get_hybrid_grammar()`

> **Применение к сигналам `emit_signal`:** для сигнальных агентов оркестратор автоматически
> ставит `json_schema` из контракта (`signals/root.schema.json`) через
> `build_signal_envelope_schema()` — свободный текст живёт в поле `thought`, а структура
> сигнала (напр. `element_1_mental`) защищена `enum`/`required`. Это гарантирует, что модель не
> исказит имена/значения полей. Правила и разбор бага «тихо упал»:
> [docs/SIGNAL_CONTRACTS.md](./SIGNAL_CONTRACTS.md).


# Методы снижения тупости моделей от грамматики:
1. Если вы используете трюк с первым ключом "thought_process": "...", просадка интеллекта у современных моделей (Llama 3, Qwen 2.5, Mistral) микроскопическая — порядка 1-3%.
Поле для мыслей (Think-before-speak schema).
Вместо того чтобы просить схему {"answer": "X"}, в грамматику добавляют обязательное текстовое поле первым: {"chain_of_thought": "строка", "answer": "X"}. Модель сначала свободно "думает" текстом внутри первого ключа JSON, а только потом выдает результат. Это частично возвращает ей интеллект.

{
  "thought_process": "string (обязательное поле для рассуждений шаг за шагом)",
  "decision": "string (маршрут)",
  "confidence": "number"
}

Избегайте жестких enum там, где нужно рассуждение
Грамматика заставляет вероятности токенов схлопываться. Если вы задаете enum: ["route_A", "route_B"], модель заперта. Дайте ей свободу в текстовых полях. Ограничивайте через enum только финальные ключи-переключатели, предварительно заставив заполнить свободное текстовое поле.


2. Двухпроходная генерация (Если задача супер-сложная)
Для задач, требующих максимального интеллекта (написание кода, сложная математика), не используйте грамматику в llama.cpp на первом этапе.
Проход 1 (Без грамматики): Даете задачу умной модели. Просите в системном промпте "в конце оберни ответ в JSON". Модель свободно рассуждает.
Проход 2 (Быстрый парсинг): Если модель сломала формат (забыла скобку), вы берете её ответ, кидаете в llama.cpp с промптом "Извлеки данные в формат" и включаете жесткую JSON Schema грамматику. Поскольку текст уже сгенерирован и думать не надо, модель (или даже маленькая модель-парсер на 8B параметров) мгновенно и безошибочно упакует его в JSON.


3. Метод хардкора (динамическое гибридное декодирование) через GBNF (поддерживать сложнее):
root ::= "<think>\n" thought-content "</think>\n" json-object
thought-content ::= [^<]*  # любой текст, пока не встретится закрывающий тег
json-object ::= "{" ... "}" # тут правила вашего JSON


Есть один случай, когда Гибридный GBNF — это мастхэв (Модели-Рассуждаторы)
Если вы используете в llama.cpp модели нового поколения с встроенным рассуждением (например, DeepSeek-R1, Qwen-QwQ или их дистилляции), то JSON Schema их реально убьет.
Эти модели натренированы на уровне токенов выдавать специальный тег <think>, потом простыню мыслей со сложным форматированием (математикой, блоками кода, списками), а потом </think>. Если вы засунете их в JSON Schema с полем "thought_process", они сойдут с ума, пытаясь запихнуть свои спец-токены и сложные конструкции в одну экранированную JSON-строку. Их интеллект рухнет.
Вот для таких моделей динамический гибридный GBNF — это действительно пушечный топ и единственный выход.

> **⚠️ `disable_reasoning` и Method 3:**
> Для агентов с **обычной** GBNF-грамматикой (без think-block) автоматически
> включается `disable_reasoning: true` (параметр `enable_thinking: false` в chat template).
> Это заставляет модель генерировать JSON напрямую в `content`, минуя `<think>` блок.
>
> **Для Method 3 агентов** (signal contracts, hybrid grammar с think-block)
> `disable_reasoning` НЕ включается — модель думает в `<think>...</think>`.
> См. `orchestrator/mod.rs:539` — флаг `uses_method_3`.

Вы можете написать простую обертку в вашем проекте. Логика такая:
Вы храните грамматику агентов в удобной JSON Schema (или Pydantic).
Перед вызовом llama.cpp ваш скрипт берет эту JSON Schema и конвертирует в GBNF-строку.
Скрипт приклеивает к этой строке заголовок для свободного размышления.


С таким подходом вы убиваете двух зайцев: вы пишете удобные JSON Schema в коде, а под капотом llama.cpp получает гибридный GBNF, который дает модели 100% свободы до тега </think>.

> **⚠️ Жизненный цикл грамматики в оркестраторе:**
> `take_pending_grammar()` — consume-and-clear операция. После первого вызова
> `generate_chat()` грамматика **удаляется** из движка. Если агент повторно вызывается
> (retry после ошибки, tool call, subagent call), грамматику нужно **перевыставлять**.
>
> Все retry-пути + continuation path используют **единую** функцию
> `ctx.restore_grammar()` (`orchestrator/dispatch.rs`), которая восстанавливает
> `active_grammar` из `RunContext`. Точки вызова:
> - thinking_no_answer retry
> - consecutive_incomplete retry
> - tool call error / success
> - MAX_TOKENS continuation
> - subagent retry


Примерно так будет выглядеть ваш универсальный генератор под капотом:
#### Шаг 1: Добавляем зависимость
Добавьте в `Cargo.toml`:
```toml
[dependencies]
schemars = "0.8"
```

#### Шаг 2: Генерируем базовый GBNF "офлайн" (Во время разработки)
Вам нужно один раз перегнать ваши Rust-структуры в GBNF и сохранить их как текстовые файлы в проекте (например, в папку `grammars/`). Вы можете сделать это через простой `#[test]`, который запустите у себя на машине 개발:

```rust
#[cfg(test)]
mod dev_tools {
    use super::*;
    use schemars::schema_for;
    use std::fs;
    use std::process::Command;

    // Структура вашего агента
    #[derive(schemars::JsonSchema)]
    pub struct AnalystResponse {
        pub decision: String,
        pub confidence: f32,
    }

    #[test]
    fn generate_base_grammars_for_release() {
        // 1. Получаем JSON-схему
        let schema = schema_for!(AnalystResponse);
        let schema_json = serde_json::to_string(&schema).unwrap();
        
        // 2. Сохраняем во временный файл
        fs::write("temp_schema.json", &schema_json).unwrap();

        // 3. Вызываем питоновский скрипт llama.cpp (работает только у вас на компе разработчика)
        let output = Command::new("python3")
            // Укажите путь к скачанному llama.cpp
            .arg("../../llama.cpp/examples/json_schema_to_grammar.py") 
            .arg("temp_schema.json")
            .output()
            .unwrap();

        let gbnf = String::from_utf8_lossy(&output.stdout).to_string();
        
        // 4. Сохраняем готовую базовую грамматику в исходники Tauri
        fs::create_dir_all("src/grammars").unwrap();
        fs::write("src/grammars/analyst_base.gbnf", gbnf).unwrap();
        
        fs::remove_file("temp_schema.json").unwrap();
    }
}
```
*Запустив `cargo test` у себя, вы получите готовый файл `analyst_base.gbnf`.*

#### Шаг 3: Используем гибридную грамматику в рантайме Tauri (Без Python!)
Теперь в основном коде вашего приложения (который полетит к пользователям) вы просто загружаете этот текстовый файл и оборачиваете его в гибридную логику `think`. 

У вас в проекте есть `minijinja`, но для простых строк хватит макроса `include_str!` и функции замены:

```rust
// Подтягиваем сгенерированную грамматику прямо в бинарник на этапе компиляции
const ANALYST_BASE_GBNF: &str = include_str!("grammars/analyst_base.gbnf");

/// Эта функция будет работать на компах юзеров мгновенно и безопасно
pub fn get_hybrid_grammar(base_gbnf: &str) -> String {
    // Меняем корень, как я показывал ранее
    let renamed = base_gbnf.replacen("root ::=", "json_object ::=", 1);

    // Добавляем теги <think>
    let wrapper = r#"root ::= "<think>\n" thought_content "</think>\n" json_object
thought_content ::= [^<]*
"#;

    format!("{}\n{}", wrapper, renamed)
}

// Пример использования при отправке запроса в llama.cpp (через reqwest)
pub async fn call_agent_analyst() {
    // Получаем супер-грамматику!
    let hybrid_gbnf = get_hybrid_grammar(ANALYST_BASE_GBNF);
    
    // Формируем HTTP-запрос к llama-server
    let payload = serde_json::json!({
        "messages": [{"role": "user", "content": "Проанализируй этот код..."}],
        "grammar": hybrid_gbnf, // <--- Отправляем GBNF на сервер llama.cpp
        "temperature": 0.7,
    });

    // let client = reqwest::Client::new();
    // client.post("http://127.0.0.1:8080/v1/chat/completions").json(&payload).send().await...
}
```
