use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;

fn main() {
    let target = env::var("TARGET").unwrap_or_default(); 
    // Пробрасываем переменную TARGET в код Rust, чтобы программа знала, как называется скачанный файл
    println!("cargo:rustc-env=TARGET={}", target);
    
    if target.contains("windows") {
        let bin_dir = Path::new("bin");
        if !bin_dir.exists() {
            fs::create_dir_all(bin_dir).expect("Не удалось создать папку bin");
        }

        // Скачивание yt-dlp
        let exe_name = format!("yt-dlp-{}.exe", target);
        let exe_path = bin_dir.join(exe_name);

        if !exe_path.exists() {
            println!("cargo:warning=Скачивание yt-dlp.exe (это может занять минуту)...");
            
            let status = Command::new("curl")
                .args(&[
                    "-L",
                    "-o",
                    exe_path.to_str().unwrap(),
                    "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe"
                ])
                .status()
                .expect("Не удалось запустить curl. Убедитесь, что у вас работает интернет.");

            if !status.success() {
                panic!("Не удалось скачать yt-dlp.exe. Код ошибки: {:?}", status.code());
            }
            println!("cargo:warning=yt-dlp.exe успешно скачан и будет встроен в приложение!");
        }

        // Скачивание node.exe
        let node_exe_name = format!("node-{}.exe", target);
        let node_exe_path = bin_dir.join(node_exe_name);

        if !node_exe_path.exists() {
            println!("cargo:warning=Скачивание node.exe (это может занять минуту)...");
            
            let status = Command::new("curl")
                .args(&[
                    "-L",
                    "-o",
                    node_exe_path.to_str().unwrap(),
                    "https://nodejs.org/dist/v20.12.2/win-x64/node.exe"
                ])
                .status()
                .expect("Не удалось запустить curl. Убедитесь, что у вас работает интернет.");

            if !status.success() {
                panic!("Не удалось скачать node.exe. Код ошибки: {:?}", status.code());
            }
            println!("cargo:warning=node.exe успешно скачан и будет встроен в приложение!");
        }
    }

    // --- СОЗДАНИЕ ЗАГЛУШКИ ИКОНКИ ---
    let icons_dir = Path::new("icons");
    if !icons_dir.exists() {
        fs::create_dir_all(icons_dir).expect("Не удалось создать папку icons");
    }
    let icon_path = icons_dir.join("icon.ico");
    
    // Структурно правильный 1x1 24bpp ICO файл (ровно 70 байт)
    let valid_ico:[u8; 70] =[
        0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 
        0x01, 0x01, 0x00, 0x00, 0x01, 0x00, 0x18, 0x00, 0x30, 0x00, 0x00, 0x00, 0x16, 0x00, 0x00, 0x00, 
        0x28, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x01, 0x00, 0x18, 0x00, 
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00
    ];
    if !icon_path.exists() {
        fs::write(icon_path, &valid_ico).expect("Не удалось создать валидную заглушку icon.ico");
    }

    tauri_build::build()
}