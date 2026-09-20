Отличная связка! Запуск локальных LLM внутри Tauri через `llama.cpp` — это одна из самых востребованных, но при этом требовательных к ресурсам задач: у ваших пользователей могут быть как Mac на M-чипах с unified memory, так и слабые офисные ПК без дискретной видеокарты.

Разберем подробно оба направления применительно к связке **Rust + Tauri + llama.cpp**.

---

## ЧАСТЬ 1. Оптимизация инференса llama.cpp внутри Tauri

Из статьи Z.ai берем три ключевых принципа: **контроль типов данных KV-кэша**, **разделение фаз Prefill/Decode** и **изоляция вычислений от потоков управления**.

### 1. Архитектура потоков: Защита интерфейса Tauri от фризов
Самая частая ошибка — вызывать C/C++ FFI инференса прямо внутри `#[tauri::command]`. 

Если вы сделаете тяжелый вызов в `async fn`, вы **заблокируете worker-поток Tokio**. В результате Tauri перестанет отвечать на IPC-команды из фронтенда, зависнет анимация и UI перестанет быть отзывчивым.

**Решение:** Выделенный OS-поток + каналы (`mpsc`).
```rust
use tauri::{AppHandle, Emitter}; // В Tauri v2; для v1: Window / app_handle.emit_all
use tokio::sync::mpsc;
use std::thread;

// Запрос от UI
struct GenerateRequest {
    prompt: String,
    channel_id: String,
}

#[tauri::command]
async fn generate_text(
    prompt: String,
    state: tauri::State<'_, InferenceController>,
) -> Result<(), String> {
    // Отправляем задачу в очередь выделенного потока инференса
    state.tx.send(prompt).await.map_err(|e| e.to_string())?;
    Ok(())
}

// Поток воркера (чистый синхронный OS-поток)
pub fn run_inference_worker(
    mut rx: mpsc::Receiver<String>, 
    app_handle: AppHandle
) {
    thread::spawn(move || {
        // Инициализируем модель один раз здесь
        let model = init_llama_model(); 

        while let Some(prompt) = rx.blocking_recv() {
            // Запуск генерации токенов
            model.generate(&prompt, |token| {
                // Стримим каждый токен обратно в WebView через событие
                let _ = app_handle.emit("llm-token", token);
                
                // Проверка флага отмены (CancellationToken), если юзер нажал "Stop"
                true // продолжать
            });
            let _ = app_handle.emit("llm-done", ());
        }
    });
}
```

---

### 2. Оптимизация памяти: Квантование KV-кэша (урок из статьи)
В статье инженеры уменьшали разрядность активаций и кэша. На клиентских машинах память — главный лимит. 

По умолчанию `llama.cpp` хранит KV-кэш в `FP16` или `FP32`. Для контекста в 8192 токена на моделях 7B/8B кэш может занять **2-4 ГБ VRAM/RAM** сверх размера самой модели!

При настройке контекста в `llama-cpp-2` (или через C API) **всегда включайте квантование KV-кэша**:

```rust
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::ggml::GgmlType;

let mut ctx_params = LlamaContextParams::default();

// Ограничьте контекст адекватным числом (не ставьте сразу 32k/128k)
ctx_params = ctx_params.with_n_ctx(Some(std::num::NonZeroU32::new(4096).unwrap()));

// КЛЮЧЕВОЙ МОМЕНТ: Квантование KV кэша в Q8_0 или Q4_0
// Это сокращает объем памяти под кэш в 2-4 раза практически без потери качества
ctx_params = ctx_params.with_type_k(GgmlType::Q8_0);
ctx_params = ctx_params.with_type_v(GgmlType::Q8_0);

// Размер батча для prefill (обработка входного промпта)
ctx_params = ctx_params.with_n_batch(512);
// Микробатч для физической обработки на GPU
ctx_params = ctx_params.with_n_ubatch(512);
```

---

### 3. Баланс Compute vs Memory (Фазы Prefill и Decode)
* **Фаза Prefill (обработка промпта):** compute-bound (упирается в вычислительную мощность). Чем больше `n_batch`, тем быстрее проглотит документ, но тем больше пиковое потребление RAM/VRAM. Для клиента `512` — оптимальный баланс.
* **Фаза Decode (генерация токенов по одному):** memory-bandwidth-bound (упирается в скорость памяти). Здесь важен выбор квантования самой модели:
  * Для большинства десктопов идеальный формат — **Q4_K_M** или **Q5_K_M**.
  * Если у пользователя только CPU: кванты `Q4_0` могут работать даже быстрее за счет SIMD/AVX2/NEON инструкций, чем сложные K-quants.

---

## ЧАСТЬ 2. Методология Dense Feedback Loop для Rust-кода

Концепция из статьи: **агент не может оптимизировать код вслепую — ему нужны точные, изолированные метрики.**

Если вы просите ИИ (Cursor, Claude Code, Aider): *«Сделай инференс быстрее»* — он выдаст абстрактный или ломающий логику код. Вам нужно выстроить вокруг проекта систему автоматической обратной связи.

```
┌─────────────────┐      Изменение кода      ┌──────────────────────┐
│  AI Coding      │ ───────────────────────> │ Rust Source (Tauri)  │
│  Agent          │ <─────────────────────── │                      │
└─────────────────┘   Метрики и трейсы       └──────────┬───────────┘
                                                        │
                      ┌─────────────────────────────────┴─────────────┐
                      ▼                                               ▼
         [1. Criterion (Benchmarks)]                     [2. Tracing (Tracy/Chrome)]
         - Токенов в секунду                             - Задержка между потоками
         - Время токенизации                             - Локи мьютексов
         - Накладные расходы IPC                         - Аллокации памяти
```

### Шаг 1. Изолируйте микробенчмарки через `criterion`
Создайте бенчмарки на узкие места, где LLM-агент может экспериментировать без запуска всего GUI Tauri:

```toml
# Cargo.toml
[dev-dependencies]
criterion = { version = "0.5", features = ["html_reports"] }

[[bench]]
name = "inference_bench"
harness = false
```

```rust
// benches/inference_bench.rs
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn bench_prompt_processing(c: &mut Criterion) {
    let mut engine = setup_test_llama_engine();
    let prompt = "Explain quantum computing in three sentences.";

    c.bench_function("prefill_50_tokens", |b| {
        b.iter(|| {
            // Замеряем только время создания KV-кэша для входного промпта
            engine.eval_prompt(black_box(prompt));
        })
    });
}

criterion_group!(benches, bench_prompt_processing);
criterion_main!(benches);
```

### Шаг 2. Добавьте трейсинг времени и блокировок (`tracing` + `tracy`)
Чтобы агент видел, где именно задержка — в FFI `llama.cpp`, сериализации JSON в Tauri IPC или локе `Mutex`:

```toml
[dependencies]
tracing = "0.1"
tracing-subscriber = "0.3"
tracing-tracy = "0.11" # Для анализа узких мест в реальном времени
```

```rust
use tracing::{info_span, info};

pub fn generate_step(&mut self) {
    let _span = info_span!("llama_decode_step").entered();
    
    {
        let _lock_span = info_span!("channel_send_wait").entered();
        // Если здесь уходит 5мс — агент поймет, что дело в перегрузке IPC Tauri!
        self.channel.send(token).unwrap();
    }
}
```

### Шаг 3. Инструкция для AI-агента (`CLAUDE.md` / `.cursorrules`)
Создайте файл правил для вашего ИИ-помощника в корне репозитория. В нем опишите протокол оптимизации:

```markdown
# Optimization Protocol for Local Inference

При оптимизации модулей `src-tauri/src/inference/`:

1. **Не запускай Tauri UI для проверки производительности.**
2. Перед изменениями запусти базовый замер:
   `cargo bench --bench inference_bench -- --save-baseline before`
3. Выполняй профилирование:
   `cargo test --test test_correctness` (убедись, что логиты/токены идентичны детерминированному тесту).
4. Запусти сравнение после изменений:
   `cargo bench --bench inference_bench -- --baseline before`
5. **Критерии успеха:**
   - Память (RSS): не должна расти более чем на 5%.
   - Время Prefill: не должно деградировать.
   - Никаких блокирующих FFI-вызовов внутри async functions (Tokio worker threads).
```

