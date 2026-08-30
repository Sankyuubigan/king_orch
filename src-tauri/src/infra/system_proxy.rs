//! Детект системного прокси Windows через WinHTTP API.
//!
//! Если прокси настроен (Kaspersky, ISP, корпоративный), устанавливает
//! переменные окружения HTTPS_PROXY/HTTP_PROXY, которые reqwest подхватит
//! автоматически (reqwest читает env vars по умолчанию).

/// Детект прокси и установка env vars. Вызывать ОДИН раз при старте (main.rs).
pub fn detect_and_set_proxy() {
    match detect_system_proxy() {
        Some(proxy) => {
            crate::infra::startup_log::append("INFO", &format!("Системный прокси обнаружен: {}", proxy));
            std::env::set_var("HTTPS_PROXY", &proxy);
            std::env::set_var("HTTP_PROXY", &proxy);
        }
        None => {
            crate::infra::startup_log::append("INFO", "Системный прокси не обнаружен (прямое соединение)");
        }
    }
}

#[cfg(windows)]
fn detect_system_proxy() -> Option<String> {
    use std::ffi::c_void;

    #[repr(C)]
    #[allow(non_snake_case)]
    struct WINHTTP_CURRENT_USER_IE_PROXY_CONFIG {
        fAutoDetect: i32,
        lpszAutoConfigUrl: *mut u16,
        lpszProxy: *mut u16,
        lpszProxyBypass: *mut u16,
    }

    extern "system" {
        fn WinHttpGetIEProxyConfigForCurrentUser(
            pProxyConfig: *mut WINHTTP_CURRENT_USER_IE_PROXY_CONFIG,
        ) -> i32;
        fn GlobalFree(hMem: *mut c_void) -> *mut c_void;
    }

    unsafe fn ptr_to_string(ptr: *mut u16) -> Option<String> {
        if ptr.is_null() {
            return None;
        }
        let mut len = 0;
        while *ptr.add(len) != 0 {
            len += 1;
        }
        if len == 0 {
            return None;
        }
        let slice = std::slice::from_raw_parts(ptr, len);
        let s = String::from_utf16_lossy(slice);
        if s.is_empty() { None } else { Some(s) }
    }

    unsafe fn free_config(config: &mut WINHTTP_CURRENT_USER_IE_PROXY_CONFIG) {
        if !config.lpszProxy.is_null() {
            GlobalFree(config.lpszProxy as *mut c_void);
        }
        if !config.lpszProxyBypass.is_null() {
            GlobalFree(config.lpszProxyBypass as *mut c_void);
        }
        if !config.lpszAutoConfigUrl.is_null() {
            GlobalFree(config.lpszAutoConfigUrl as *mut c_void);
        }
    }

    unsafe {
        let mut config: WINHTTP_CURRENT_USER_IE_PROXY_CONFIG = std::mem::zeroed();
        let result = WinHttpGetIEProxyConfigForCurrentUser(&mut config);
        if result == 0 {
            return None;
        }

        let raw = match ptr_to_string(config.lpszProxy) {
            Some(s) => s,
            None => {
                free_config(&mut config);
                return None;
            }
        };

        free_config(&mut config);

        parse_proxy_string(&raw)
    }
}

/// Парсинг строки прокси Windows:
///   `"http=proxy:port;https=proxy:port"` → Some("http://proxy:port")
///   `"proxy:port"`                       → Some("http://proxy:port")
///   `"socks=proxy:port"`                 → Some("socks5://proxy:port")
///   `""` / пустое                        → None
fn parse_proxy_string(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() || raw.eq_ignore_ascii_case("direct") {
        return None;
    }

    // Формат: "key=value;key=value" (напр. "http=proxy:80;https=proxy:80")
    for part in raw.split(';') {
        let part = part.trim();
        if let Some(proxy_val) = part.strip_prefix("https=") {
            return Some(normalize_proxy_url(proxy_val));
        }
    }
    // Нет "https=" — пробуем "socks="
    for part in raw.split(';') {
        let part = part.trim();
        if let Some(proxy_val) = part.strip_prefix("socks=") {
            return Some(normalize_proxy_url(proxy_val));
        }
    }
    // Нет "https=" и "socks=" — пробуем "http="
    for part in raw.split(';') {
        let part = part.trim();
        if let Some(proxy_val) = part.strip_prefix("http=") {
            return Some(normalize_proxy_url(proxy_val));
        }
    }
    // Просто "host:port" без префикса — считаем HTTP-прокси
    if raw.contains(':') && !raw.starts_with("http://") && !raw.starts_with("socks") {
        return Some(format!("http://{}", raw));
    }
    // Уже URL?
    if raw.starts_with("http://") || raw.starts_with("https://") || raw.starts_with("socks") {
        return Some(raw.to_string());
    }
    None
}

fn normalize_proxy_url(val: &str) -> String {
    let val = val.trim();
    if val.starts_with("http://") || val.starts_with("https://") || val.starts_with("socks") {
        val.to_string()
    } else {
        format!("http://{}", val)
    }
}

#[cfg(not(windows))]
fn detect_system_proxy() -> Option<String> {
    // На Linux/macOS прокси обычно задаётся через env vars — reqwest уже читает их.
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_proxy_https_only() {
        assert_eq!(parse_proxy_string("https=proxy.example.com:8080"), Some("http://proxy.example.com:8080".into()));
    }

    #[test]
    fn parse_proxy_both() {
        assert_eq!(
            parse_proxy_string("http=proxy:3128;https=proxy:3128"),
            Some("http://proxy:3128".into())
        );
    }

    #[test]
    fn parse_proxy_socks() {
        assert_eq!(
            parse_proxy_string("socks=127.0.0.1:1080"),
            Some("socks5://127.0.0.1:1080".into())
        );
    }

    #[test]
    fn parse_proxy_plain() {
        assert_eq!(parse_proxy_string("proxy:3128"), Some("http://proxy:3128".into()));
    }

    #[test]
    fn parse_proxy_direct() {
        assert_eq!(parse_proxy_string("direct"), None);
    }

    #[test]
    fn parse_proxy_empty() {
        assert_eq!(parse_proxy_string(""), None);
    }

    #[test]
    fn parse_proxy_full_url() {
        assert_eq!(
            parse_proxy_string("http://proxy.example.com:3128"),
            Some("http://proxy.example.com:3128".into())
        );
    }
}
