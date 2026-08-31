//! Диагностика сети при старте приложения.
//!
//! Проверяет: DNS-резолв, TCP-соединение, системный прокси.
//! Результаты пишутся в startup_log для отладки проблем с доступностью GitHub.

use std::time::{Duration, Instant};

const TEST_HOSTS: &[(&str, u16)] = &[
    ("api.github.com", 443),
    ("github.com", 443),
    ("objects.githubusercontent.com", 443),
];

/// Запустить полную диагностику сети. Вызывать ОДИН раз при старте (main.rs).
pub fn run_diagnostics() {
    log_system_proxy();
    for &(host, port) in TEST_HOSTS {
        check_dns(host);
        check_tcp(host, port);
    }
}

fn log_system_proxy() {
    #[cfg(windows)]
    {
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

        unsafe {
            let mut config: WINHTTP_CURRENT_USER_IE_PROXY_CONFIG = std::mem::zeroed();
            let result = WinHttpGetIEProxyConfigForCurrentUser(&mut config);
            if result != 0 {
                let proxy = ptr_to_string(config.lpszProxy);
                let bypass = ptr_to_string(config.lpszProxyBypass);
                let auto_url = ptr_to_string(config.lpszAutoConfigUrl);

                let proxy_str = proxy.unwrap_or_else(|| "(нет)".to_string());
                let bypass_str = bypass.unwrap_or_else(|| "(нет)".to_string());
                let auto_str = auto_url.unwrap_or_else(|| "(нет)".to_string());

                crate::infra::startup_log::append(
                    "NET",
                    &format!(
                        "Proxy: {} | Bypass: {} | AutoConfig: {} | AutoDetect: {}",
                        proxy_str, bypass_str, auto_str, config.fAutoDetect
                    ),
                );

                if !config.lpszProxy.is_null() {
                    GlobalFree(config.lpszProxy as *mut c_void);
                }
                if !config.lpszProxyBypass.is_null() {
                    GlobalFree(config.lpszProxyBypass as *mut c_void);
                }
                if !config.lpszAutoConfigUrl.is_null() {
                    GlobalFree(config.lpszAutoConfigUrl as *mut c_void);
                }
            } else {
                crate::infra::startup_log::append("NET", "WinHttpGetIEProxyConfigForCurrentUser: FAILED");
            }
        }
    }
    #[cfg(not(windows))]
    {
        let proxy = std::env::var("HTTPS_PROXY")
            .or_else(|_| std::env::var("HTTP_PROXY"))
            .unwrap_or_else(|_| "(нет)".to_string());
        crate::infra::startup_log::append("NET", &format!("Proxy (env): {}", proxy));
    }
}

fn check_dns(host: &str) {
    use std::net::ToSocketAddrs;

    let start = Instant::now();
    match format!("{}:443", host).to_socket_addrs() {
        Ok(addrs) => {
            let ips: Vec<String> = addrs.map(|a| a.ip().to_string()).collect();
            let elapsed = start.elapsed();
            crate::infra::startup_log::append(
                "NET",
                &format!("DNS {} → {} ({:?})", host, ips.join(", "), elapsed),
            );
        }
        Err(e) => {
            let elapsed = start.elapsed();
            crate::infra::startup_log::append(
                "NET",
                &format!("DNS {} → FAILED: {} ({:?})", host, e, elapsed),
            );
        }
    }
}

fn check_tcp(host: &str, port: u16) {
    use std::net::TcpStream;

    let addr = format!("{}:{}", host, port);
    let start = Instant::now();
    match TcpStream::connect_timeout(
        &addr.parse().unwrap_or_else(|_| {
            use std::net::ToSocketAddrs;
            addr.to_socket_addrs()
                .expect("DNS resolved")
                .next()
                .expect("at least one addr")
        }),
        Duration::from_secs(5),
    ) {
        Ok(_stream) => {
            let elapsed = start.elapsed();
            crate::infra::startup_log::append(
                "NET",
                &format!("TCP {}:{} → OK ({:?})", host, port, elapsed),
            );
        }
        Err(e) => {
            let elapsed = start.elapsed();
            crate::infra::startup_log::append(
                "NET",
                &format!("TCP {}:{} → FAILED: {} ({:?})", host, port, e, elapsed),
            );
        }
    }
}
