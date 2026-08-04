fn main() {
    // Передаем TARGET триплет в Rust код на этапе компиляции
    let target = std::env::var("TARGET").unwrap_or_else(|_| "unknown".to_string());
    println!("cargo:rustc-env=TARGET={}", target);

    // ── Отложенная загрузка CUDA DLL (Delay-Load) ──
    // Приложение запускается БЕЗ CUDA Toolkit: импорты CUDA откладываются,
    // main() и Tauri UI работают сразу, а cublas64_12.dll и др. подгружаются
    // при первом вызове CUDA-функций (после установки движка llamacpp).
    if target.contains("windows-msvc") {
        for dll in ["cudart64_12.dll", "cublas64_12.dll", "cublasLt64_12.dll"] {
            println!("cargo:rustc-link-arg=/DELAYLOAD:{}", dll);
        }
        // delayimp.lib — поддержка механизма отложенной загрузки
        println!("cargo:rustc-link-lib=delayimp");
    }

    tauri_build::build()
}