fn main() {
    // Передаем TARGET триплет в Rust код на этапе компиляции
    let target = std::env::var("TARGET").unwrap_or_else(|_| "unknown".to_string());
    println!("cargo:rustc-env=TARGET={}", target);
    
    tauri_build::build()
}