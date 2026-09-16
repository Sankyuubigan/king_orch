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
    // Единый log::Log из tauri-plugin-logs (core rules §2.5): файл king_orch.log
    // РЯДОМ С EXE (юзер может прислать его, даже если приложение не открывается
    // или падает на старте), dev-зеркало test/last_logs.txt и вкладка «Логи».
    // Ранний pre-Tauri период пишется через early_* (краш-лог живёт сразу).
    tauri_plugin_logs::early_init("king_orch.log");

    tauri_plugin_logs::early_log(
        "INFO",
        &format!("=== King Orch {}: запуск ===", env!("CARGO_PKG_VERSION")),
    );
    tauri_plugin_logs::early_log(
        "INFO",
        &format!(
            "exe: {}",
            std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default()
        ),
    );
    tauri_plugin_logs::early_log(
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
    tauri_plugin_logs::early_log(
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
    tauri_plugin_logs::early_log("INFO", "Tauri: создание приложения…");

    // ── WebView2: программный рендер UI (без GPU-процесса) ──
    // Окно в фоне под нагрузкой GPU (llama.cpp + другие программы) может
    // показывать белый/непрорисованный буфер из-за перезапуска GPU-процесса
    // WebView2. Перевод UI на софтварный композитинг (--disable-gpu) убирает
    // эту зависимость (сам llama.cpp живёт в отдельном процессе и грузит GPU).
    // ВАЖНО: переменная окружения WEBVIEW2_ADDITIONAL_BROWSER_ARGS WebView2
    // ИГНОРИРУЕТ, т.к. wry сам задаёт доп. аргументы (см. build.bat). Поэтому
    // флаг --disable-gpu пробрасывается через additionalBrowserArgs в
    // tauri.conf.json (и в dev-override, подключаемом через build.bat), а не через env.

    // ── Телеметрия: решение принимаем ДО создания Tauri-приложения ──
    // Настройка «Отправлять анонимные отчёты об ошибках» (по умолчанию
    // включена). Плагин логов регистрируется ВСЕГДА (вкладка «Логи» нужна в
    // любом случае), а облачная отправка управляется флагом set_reporting_enabled:
    // если юзер снял галочку — отправка в Aptabase физически блокируется.
    let telemetry_enabled = infra::config::load_config_early().allow_error_reports;
    tauri_plugin_logs::early_log(
        "INFO",
        if telemetry_enabled {
            "Телеметрия: включена (анонимные отчёты об ошибках)"
        } else {
            "Телеметрия: ОТКЛЮЧЕНА пользователем в настройках"
        },
    );

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_about_updates::init())
        .plugin(tauri_plugin_logs::init())

        .manage(AppState {
            cancel_flag: Arc::new(AtomicBool::new(false)),
        })
        .setup(move |app| {
            log::info!("setup(): начало");
            let app_handle = app.handle();

            // 🔐 Форвардинг запросов разрешений в UI (плашка с 3 кнопками).
            api::permissions::init_permission_forwarding(&app_handle);

            // 🔔 Форвардинг уведомлений о VRAM в UI (non-blocking, одна кнопка ОК).
            api::vram::init_vram_forwarding(&app_handle);

            // Облачная отправка: блокируем, если юзер снял галочку. Плагин логов
            // поднял своё reporting-состояние из tauri.conf.json при регистрации.
            if !telemetry_enabled {
                tauri_plugin_logs::set_reporting_enabled(false);
            }
            tauri_plugin_logs::track_event("app_started", None);

            let _ = infra::session_manager::sessions_dir(&app_handle);
            log::info!("setup(): сессии и чат-лог готовы");

            // ── 🛡 Авто-чистка «отравленных» конфигов ──
            // Легаси-версии могли добавить mmproj (мультимодальный ПРОЕКТОР) в
            // список моделей/активную модель. Запуск проектора как LLM валит
            // llama-server. Игнорируем такие записи на старте (см. is_mmproj_file).
            // Модели удаляются из списка, но файл юзера на диске не трогаем.
            {
                let mut cfg = infra::load_config(&app_handle);
                let removed: Vec<String> = cfg
                    .models
                    .iter()
                    .filter(|m| infra::is_mmproj_file(m))
                    .cloned()
                    .collect();
                if !removed.is_empty() {
                    cfg.models.retain(|m| !infra::is_mmproj_file(m));
                    if let Some(last) = &cfg.last_model {
                        if infra::is_mmproj_file(last) {
                            cfg.last_model = None;
                        }
                    }
                    infra::save_config(&app_handle, &cfg);
                    for m in &removed {
                        log::warn!(
                            "setup(): удалён mmproj из списка моделей: {} (файл не тронут)",
                            m
                        );
                    }
                }
            }

            // ── Новая архитектура: движок llama.cpp — ОТДЕЛЬНЫЙ процесс ──
            // Приложение НЕ линкует llama.cpp нативно (нет PE-импортов и DLL
            // рядом с exe). Инференс идёт через llama-server.exe по HTTP,
            // поэтому на старте нужен только сам движок в папке <exe>/llamacpp.
            let engine_dir = api::llamacpp::get_engine_dir(&app_handle);
            if infra::llamacpp_installer::has_any_installed(&engine_dir) {
                log::info!("setup(): движок llama.cpp найден");
            } else {
                log::info!("setup(): движок llama.cpp НЕ установлен (инференс будет недоступен до установки)");
            }
            let app_for_update = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                let _ = api::llamacpp::check_engine_update(app_for_update).await;
            });

            log::info!("setup(): OK");

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
                                log::info!("[WV] Focused({})", f);
                                if f {
                                    let _ = cb_win.eval("void document.documentElement.offsetHeight");
                                }
                            }
                            tauri::WindowEvent::Destroyed => {
                                log::info!("[WV] Destroyed");
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
            api::test::get_pipeline_test_list,
            api::test::run_pipeline_test_cmd,
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

    log::info!("Приложение закрыто");
}