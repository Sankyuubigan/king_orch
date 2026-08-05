//! Детекция NVIDIA GPU и версии CUDA драйвера.
//! Порядок: NVML (уже используется проектом) → nvidia-smi → нет GPU.

use std::process::Command;

/// Минимальная мажорная версия CUDA драйвера для работы нашего движка
/// (exe слинкован с cublas64_12.dll — нужен драйвер с поддержкой CUDA 12).
pub const MIN_CUDA_MAJOR: u32 = 12;

#[derive(Debug, Clone)]
pub struct GpuInfo {
    pub has_nvidia: bool,
    pub gpu_name: String,
    /// Мажорная версия CUDA драйвера (0 = не определена)
    pub cuda_major: u32,
    /// Минорная версия CUDA драйвера
    pub cuda_minor: u32,
    pub driver_version: String,
}

impl Default for GpuInfo {
    fn default() -> Self {
        Self {
            has_nvidia: false,
            gpu_name: String::new(),
            cuda_major: 0,
            cuda_minor: 0,
            driver_version: String::new(),
        }
    }
}

/// Определяет видеокарту: NVML → nvidia-smi.
pub fn detect_gpu() -> GpuInfo {
    if let Ok(info) = detect_via_nvml() {
        return info;
    }
    if let Ok(info) = detect_via_nvidia_smi() {
        return info;
    }
    GpuInfo::default()
}

/// Драйвер поддерживает CUDA 12+ (т.е. наш движок сможет загрузиться)
pub fn supports_cuda12(info: &GpuInfo) -> bool {
    info.has_nvidia && info.cuda_major >= MIN_CUDA_MAJOR
}

/// Драйвер есть, но слишком старый (CUDA 11.x) — нужен апгрейд драйвера
pub fn requires_driver_update(info: &GpuInfo) -> bool {
    info.has_nvidia && info.cuda_major > 0 && info.cuda_major < MIN_CUDA_MAJOR
}

fn detect_via_nvml() -> Result<GpuInfo, String> {
    let nvml = nvml_wrapper::Nvml::init().map_err(|e| format!("NVML init: {}", e))?;
    let device = nvml.device_by_index(0).map_err(|e| format!("NVML device: {}", e))?;
    let name = device.name().unwrap_or_else(|_| "NVIDIA GPU".to_string());
    let driver = nvml
        .sys_driver_version()
        .map_err(|e| format!("NVML driver: {}", e))?;
    let cuda_ver = nvml
        .sys_cuda_driver_version()
        .map_err(|e| format!("NVML cuda: {}", e))?;
    let major = (cuda_ver / 1000) as u32;
    let minor = ((cuda_ver % 1000) / 10) as u32;
    Ok(GpuInfo {
        has_nvidia: true,
        gpu_name: name,
        cuda_major: major,
        cuda_minor: minor,
        driver_version: driver,
    })
}

fn detect_via_nvidia_smi() -> Result<GpuInfo, String> {
    let mut cmd = Command::new("nvidia-smi");
    #[cfg(target_os = "windows")]
    { use std::os::windows::process::CommandExt; cmd.creation_flags(0x08000000); }
    let output = cmd
        .output()
        .map_err(|e| format!("nvidia-smi not found: {}", e))?;
    if !output.status.success() {
        return Err(format!("nvidia-smi exit: {}", output.status));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut cuda_major = 0u32;
    let mut cuda_minor = 0u32;
    for line in stdout.lines() {
        if let Some(idx) = line.find("CUDA Version:") {
            let ver_str = line[idx + "CUDA Version:".len()..].trim();
            let parts: Vec<&str> = ver_str.split('.').collect();
            cuda_major = parts
                .first()
                .and_then(|s| s.trim().parse::<u32>().ok())
                .unwrap_or(0);
            cuda_minor = parts
                .get(1)
                .and_then(|s| s.trim().parse::<u32>().ok())
                .unwrap_or(0);
            break;
        }
    }
    if cuda_major == 0 {
        return Err("CUDA version not found in nvidia-smi output".to_string());
    }
    Ok(GpuInfo {
        has_nvidia: true,
        gpu_name: "NVIDIA GPU (nvidia-smi)".to_string(),
        cuda_major,
        cuda_minor,
        driver_version: String::new(),
    })
}

/// Человекочитаемое описание статуса GPU для логов/UI
pub fn describe_gpu(info: &GpuInfo) -> String {
    if !info.has_nvidia {
        "NVIDIA GPU не обнаружен. Будет использован CPU-режим.".to_string()
    } else if requires_driver_update(info) {
        format!(
            "Обнаружен NVIDIA GPU ({}), но драйвер поддерживает только CUDA {}.{}.\n\
             Для GPU-ускорения обновите драйвер NVIDIA до версии >= 527.41 (CUDA 12+).\n\
             Пока работаем в CPU-режиме.",
            info.gpu_name, info.cuda_major, info.cuda_minor
        )
    } else {
        format!(
            "NVIDIA GPU: {} (драйвер CUDA {}.{}) — GPU-ускорение доступно.",
            info.gpu_name, info.cuda_major, info.cuda_minor
        )
    }
}
