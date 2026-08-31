#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Тонкий бутстраппер — только подключает слои и запускает Tauri.
//! Вся логика изолирована в слоях: api, domain, infra.

mod api;
mod domain;
mod infra;

use api::AppState;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    // ── Логирование с первой миллисекунды запуска ──
    // Лог пишется РЯДОМ С EXE (king_orch.log), чтобы юзер мог прислать его,
    // даже если приложение не открывается или падает на старте.
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    infra::startup_log::init(&exe_dir);
    infra::startup_log::install_panic_hook();

    infra::startup_log::append(
        "INFO",
        &format!("=== King Orch {}: запуск ===", env!("CARGO_PKG_VERSION")),
    );
    infra::startup_log::append(
        "INFO",
        &format!(
            "exe: {}",
            std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default()
        ),
    );
    infra::startup_log::append(
        "INFO",
        &format!(
            "OS: {} | arch: {} | CPU: {}",
            std::env::var("OS").unwrap_or_default(),
            std::env::consts::ARCH,
            std::env::var("PROCESSOR_IDENTIFIER").unwrap_or_default(),
        ),
    );

    // ── Системный прокси: детект до любых HTTP-запросов ──
    infra::system_proxy::detect_and_set_proxy();

    // ── Диагностика сети: DNS, TCP, proxy ──
    infra::network_diagnostics::run_diagnostics();
    let gpu = infra::gpu_detector::detect_gpu();
    infra::startup_log::append(
        "INFO",
        &format!(
            "GPU: {} | CUDA драйвер: {}.{} | compute: {}.{} | нужен вариант: {}",
            if gpu.gpu_name.is_empty() { "не обнаружена" } else { &gpu.gpu_name },
            gpu.cuda_major,
            gpu.cuda_minor,
            gpu.compute_major,
            gpu.compute_minor,
            infra::llamacpp_installer::select_variant(),
        ),
    );
    infra::startup_log::append("INFO", "Tauri: создание приложения…");

    // ── WebView2: программный рендер UI (без GPU-процесса) ──
    // Окно в фоне под нагрузкой GPU (llama.cpp + другие программы) может
    // показывать белый/непрорисованный буфер из-за перезапуска GPU-процесса
    // WebView2. Перевод UI на софтварный композитинг (--disable-gpu) убирает
    // эту зависимость (сам llama.cpp живёт в отдельном процессе и грузит GPU).
    // ВАЖНО: переменная окружения WEBVIEW2_ADDITIONAL_BROWSER_ARGS WebView2
    // ИГНОРИРУЕТ, т.к. wry сам задаёт доп. аргументы (см. build.cjs). Поэтому
    // флаг --disable-gpu пробрасывается через additionalBrowserArgs в
    // tauri.conf.json (и в dev-override в build.cjs), а не через env.

    // ── Телеметрия: решение принимаем ДО создания Tauri-приложения ──
    // Читаем настройку «Отправлять анонимные отчёты об ошибках» (по умолчанию
    // включена). Если юзер её снял — плагин Aptabase вообще не регистрируется,
    // поэтому отправка данных физически невозможна.
    let telemetry_enabled = infra::config::load_config_early().allow_error_reports;
    infra::startup_log::append(
        "INFO",
        if telemetry_enabled {
            "Телеметрия: включена (анонимные отчёты об ошибках)"
        } else {
            "Телеметрия: ОТКЛЮЧЕНА пользователем в настройках"
        },
    );

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_about_updates::init());

    // Плагин телеметрии подключаем ТОЛЬКО при разрешении пользователя.
    let builder = if telemetry_enabled {
        builder.plugin(infra::telemetry::install_plugin())
    } else {
        builder
    };

    builder
        .manage(AppState {
            cancel_flag: Arc::new(AtomicBool::new(false)),
        })
        .setup(move |app| {
            infra::startup_log::append("INFO", "setup(): начало");
            let app_handle = app.handle();

            // 🔐 Форвардинг запросов разрешений в UI (плашка с 3 кнопками).
            api::permissions::init_permission_forwarding(&app_handle);

            // Телеметрия: инициализация только если юзер не против.
            if telemetry_enabled {
                infra::telemetry::init(&app_handle);
                infra::telemetry::track_event("app_started", serde_json::Value::Null);
            }

            let _ = infra::session_manager::sessions_dir(&app_handle);
            api::chat::init_log_file();
            infra::startup_log::append("INFO", "setup(): сессии и чат-лог готовы");

            // ── Новая архитектура: движок llama.cpp — ОТДЕЛЬНЫЙ процесс ──
            // Приложение НЕ линкует llama.cpp нативно (нет PE-импортов и DLL
            // рядом с exe). Инференс идёт через llama-server.exe по HTTP,
            // поэтому на старте нужен только сам движок в папке <exe>/llamacpp.
            let engine_dir = api::llamacpp::get_engine_dir(&app_handle);
            if infra::llamacpp_installer::has_any_installed(&engine_dir) {
                infra::startup_log::append("INFO", "setup(): движок llama.cpp найден");
            } else {
                infra::startup_log::append("INFO", "setup(): движок llama.cpp НЕ установлен (инференс будет недоступен до установки)");
            }
            let app_for_update = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                let _ = api::llamacpp::check_engine_update(app_for_update).await;
            });

            infra::startup_log::append("INFO", "setup(): OK");

            // ── Диагностика + страховка WebView2 окна ──
            // Логируем события окна (фокус/закрытие) в локальный лог и
            // принудительно перерисовываем при возврате фокуса, чтобы окно не
            // оставалось белым (глюк compositor под нагрузкой GPU).
            // Важно: on_window_event вешается на САМО окно (WebviewWindow), а не
            // на App; события Occluded в WebView2/Tauri нет — только Focused,
            // Destroyed и пр. (см. docs.rs tauri::WindowEvent).
            {
                use tauri::Manager;
                if let Some(win) = app.get_webview_window("main") {
                    // Отдельный клон для замыкания: сам `win` борруется методом
                    // on_window_event(&self), а клон перемещается в замыкание.
                    let cb_win = win.clone();
                    win.on_window_event(move |event| {
                        match event {
                            tauri::WindowEvent::Focused(focused) => {
                                let f = *focused;
                                infra::startup_log::append("WV", &format!("Focused({})", f));
                                if f {
                                    let _ = cb_win.eval("void document.documentElement.offsetHeight");
                                }
                            }
                            tauri::WindowEvent::Destroyed => {
                                infra::startup_log::append("WV", "Destroyed");
                            }
                            _ => {}
                        }
                    });
                }
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            api::config::get_config,
            api::config::set_config_value,
            api::config::set_last_model,
            api::config::set_theme,
            api::config::set_prompt_format,
            api::agents::get_agents,
            api::sessions::get_sessions,
            api::sessions::load_session,
            api::sessions::save_session,
            api::sessions::delete_session,
            api::sessions::rename_session,
            api::sessions::open_session_folder,
            api::models::get_models_catalog,
            api::models::get_model_params,
            api::models::set_model_params,
            api::models::reset_model_params,
            api::models::add_model,
            api::models::remove_model,
            api::models::delete_model_file,
            api::models::get_mmproj_path,
            api::models::get_model_capabilities,
            api::models::get_all_capabilities,
            api::models::ensure_mmproj,
            api::models::get_auto_download_info,
            api::models::auto_download_default_model,
            api::chat::chat_request,
            api::chat::stop_processing,
            api::permissions::respond_permission,
            api::chat::get_prompt_preview,
            api::chat::get_prompt_memory,
            api::graph::read_workflow_file,
            api::graph::save_workflow,
            api::test::run_iterative_test,
            api::test::read_test_file,
            api::test::write_test_results,
            api::coding_test::get_coding_bench_info,
            api::coding_test::run_coding_bench,
            infra::downloader::download_model,
            api::file_utils::write_text_file,
            api::file_utils::read_text_file,
            api::llamacpp::get_engine_status,
            api::llamacpp::install_llamacpp,
            api::llamacpp::set_engine_variant,
            api::llamacpp::check_engine_update,
            api::llamacpp::install_engine_update,
            api::llamacpp::remove_engine,
            api::llamacpp::set_engine_dir,
            api::telemetry::track_error,
            api::log_frontend_event,
            api::translate::translate_message,
            api::updater::check_github_release_update,
            api::updater::install_update_from_github,
        ])
        .build(tauri::generate_context!())
        .expect("ошибка создания приложения Tauri")
        .run(|_app_handle, event| {
            // Гарантированная зачистка движка llama.cpp (llama-server.exe) при
            // выходе из приложения: на Windows дочерний процесс не убивается
            // вместе с родителем и «висит» в памяти. Дополнительно к этому
            // LlamaEngine назначается в Windows Job Object с KILL_ON_JOB_CLOSE.
            if let tauri::RunEvent::ExitRequested { .. } = event {
                crate::infra::process_util::kill_active_engines();
            }
        });

    infra::startup_log::append("INFO", "Приложение закрыто");
}