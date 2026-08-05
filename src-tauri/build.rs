use std::time::Duration;

fn main() {
    let target = std::env::var("TARGET").unwrap_or_else(|_| "unknown".to_string());
    println!("cargo:rustc-env=TARGET={}", target);

    // tauri_build::build() может падать с os error 5/32 от Windows Defender на
    // свежесозданных файлах. Используем try_build с ретраями.
    let mut last_error: Option<String> = None;
    for attempt in 0..12u32 {
        match tauri_build::try_build(Default::default()) {
            Ok(()) => {
                last_error = None;
                break;
            }
            Err(e) => {
                last_error = Some(format!("{e:#}"));
                println!("cargo:warning=king_orch: try_build attempt {} failed: {e:#}", attempt + 1);
                if attempt < 11 {
                    std::thread::sleep(Duration::from_millis(1500 * (attempt as u64 + 1)));
                }
            }
        }
    }
    if let Some(err) = last_error {
        panic!("{err}");
    }
}
