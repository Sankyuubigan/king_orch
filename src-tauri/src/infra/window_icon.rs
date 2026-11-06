//! 🪟 Иконка панели задач: выставляем ICON_BIG/ICON_SMALL ДО показа окна.
//!
//! Кнопка панели задач Windows запрашивает иконки окна (WM_GETICON) один раз —
//! в момент создания кнопки — и больше их не перечитывает. Tauri-runtime
//! выставляет только ICON_SMALL (заголовок), ICON_BIG остаётся NULL, и кнопка
//! рисует фолбэк Explorer — shell-иконку по пути exe из кэша. В дев-сборке exe
//! заменяется при каждой пересборке → кэш протухает → плавающая заглушка
//! (диагностика 06.11.26, tasks/261104_2259-6ec77ce6/handoff.md).
//!
//! Решение: окно создаётся скрытым (`visible: false`), здесь на него
//! выставляются обе иконки из ресурсов самого exe, после чего окно показывают —
//! кнопка создаётся уже с нашей иконкой, кэш Explorer больше ни при чём.
//!
//! HICON, переданный в WM_SETICON, принадлежит окну (система освободит при
//! уничтожении); хэндлы живут до смерти процесса — выставляются один раз.

#[cfg(windows)]
use {
    std::os::windows::ffi::OsStrExt,
    windows_sys::Win32::Foundation::HWND,
    windows_sys::Win32::UI::WindowsAndMessaging::{
        PrivateExtractIconsW, SendMessageTimeoutW, HICON, ICON_BIG, ICON_SMALL, SMTO_ABORTIFHUNG,
        WM_SETICON,
    },
};

/// Выставить окну иконки SMALL (заголовок) и BIG (кнопка панели задач) из
/// ресурсов текущего exe. `hwnd` — дескриптор окна (`win.hwnd()` как isize).
/// Вызывается ДО `show()` окна: кнопка панели создаётся при показе, когда
/// ICON_BIG уже стоит. Ошибка не фатальна для старта — вызывающий пишет
/// `log::warn!` и всё равно показывает окно (деградация = прежнее поведение).
pub fn apply_window_icons(hwnd: isize) -> Result<(), String> {
    #[cfg(windows)]
    {
        let exe = std::env::current_exe()
            .map_err(|e| format!("current_exe не удался: {e}"))?
            .display()
            .to_string();
        log::info!("[WINDOW_ICON] hwnd={hwnd:#x} извлекаю иконки из {}", exe);

        // Широкая (UTF-16) строка пути с нуль-терминатором.
        let mut exe_w: Vec<u16> = std::ffi::OsStr::new(&exe)
            .encode_wide()
            .collect();
        exe_w.push(0);

        let small = extract_icon(&exe_w, 32)?;
        let big = extract_icon(&exe_w, 256)?;
        log::info!(
            "[WINDOW_ICON] извлечены HICON: SMALL 32x32 = {small:p}, BIG 256x256 = {big:p}"
        );

        send_set_icon(hwnd, ICON_SMALL, small, "SMALL")?;
        send_set_icon(hwnd, ICON_BIG, big, "BIG")?;
        log::info!("[WINDOW_ICON] WM_SETICON SMALL+BIG доставлены окну {hwnd:#x}");
        Ok(())
    }

    #[cfg(not(windows))]
    {
        let _ = hwnd; // не Windows: иконка задаётся средой, делать нечего
        Ok(())
    }
}

/// Извлечь из exe один HICON заданного размера (первая иконка-ресурс).
#[cfg(windows)]
fn extract_icon(exe_wide: &[u16], size: i32) -> Result<HICON, String> {
    let mut hicon: HICON = std::ptr::null_mut();
    let mut icon_id: u32 = 0;
    let n = unsafe {
        PrivateExtractIconsW(
            exe_wide.as_ptr(),
            0,
            size,
            size,
            &mut hicon,
            &mut icon_id,
            1,
            0,
        )
    };
    if n < 1 || hicon.is_null() {
        return Err(format!("PrivateExtractIconsW {size}x{size}: не извлекли (n={n})"));
    }
    Ok(hicon)
}

/// Доставить WM_SETICON окну с таймаутом (окно могло зависнуть — не ждём вечно).
#[cfg(windows)]
fn send_set_icon(hwnd: isize, kind: u32, hicon: HICON, label: &str) -> Result<(), String> {
    let mut result: usize = 0;
    let sent = unsafe {
        SendMessageTimeoutW(
            hwnd as HWND,
            WM_SETICON,
            kind as usize,
            hicon as isize,
            SMTO_ABORTIFHUNG,
            1000,
            &mut result,
        )
    };
    if sent == 0 {
        return Err(format!("WM_SETICON {label}: SendMessageTimeoutW не доставил"));
    }
    Ok(())
}
