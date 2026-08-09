//! Детекция NVIDIA GPU, версии CUDA драйвера и вычислительной способности (compute capability).
//! Порядок: NVML (уже используется проектом) → nvidia-smi → нет GPU.

use std::process::Command;

/// Минимальная мажорная версия CUDA драйвера для работы нашего движка
/// (exe слинкован с cublas64_12.dll — нужен драйвер с поддержкой CUDA 12).
pub const MIN_CUDA_MAJOR: u32 = 12;

/// Поколение CUDA-сборки движка llama.cpp, требуемое для GPU.
/// Сборка `cuda-12.4` НЕ содержит ядер Blackwell (sm_120, RTX 50xx) —
/// они появились только в сборках CUDA >= 12.8 (см. ggml/src/ggml-cuda/CMakeLists.txt:
/// "# 120 == Blackwell, needs CUDA v12.8"). Для Blackwell нужен вариант `cuda-13.x`.
/// Сборка `cuda-13.x` (в отличие от мифа «только для 50xx») содержит ядра
/// sm_75..sm_120 — включая RTX 40xx (sm_89). Выбор между cuda-12/13 на не-Blackwell
/// определяется свежестью драйвера, как в Jan (см. janhq/jan backend.rs,
/// `get_supported_features`: на Windows cuda-13 доступен при драйвере >= 580).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CudaGen {
    /// Сборка cuda-12.4 — для драйверов CUDA 12.x (sm_5x..sm_11x, все RTX до 40xx)
    Cuda12,
    /// Сборка cuda-13.x — для Blackwell (sm_120) и свежих драйверов CUDA 13+ (R580+)
    Cuda13,
}

impl CudaGen {
    pub fn label(self) -> &'static str {
        match self {
            CudaGen::Cuda12 => "cuda-12.x",
            CudaGen::Cuda13 => "cuda-13.x",
        }
    }
}

#[derive(Debug, Clone)]
pub struct GpuInfo {
    pub has_nvidia: bool,
    pub gpu_name: String,
    /// Мажорная версия CUDA драйвера (0 = не определена)
    pub cuda_major: u32,
    /// Минорная версия CUDA драйвера
    pub cuda_minor: u32,
    pub driver_version: String,
    /// Мажорный номер compute capability (sm_XX), 0 = не определён.
    /// Blackwell (RTX 50xx) = 12, RTX 30/40xx = 8/9, GTX 10xx = 6.
    pub compute_major: u32,
    /// Минорный номер compute capability (sm_XX.Y)
    pub compute_minor: u32,
}

impl Default for GpuInfo {
    fn default() -> Self {
        Self {
            has_nvidia: false,
            gpu_name: String::new(),
            cuda_major: 0,
            cuda_minor: 0,
            driver_version: String::new(),
            compute_major: 0,
            compute_minor: 0,
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

/// Какую сборку движка (cuda-12.4 / cuda-13.x) требует установленная GPU.
/// None = CUDA не используется (нет NVIDIA / старый драйвер / нет данных).
///
/// Правила (обновлены по логике Jan, см. `janhq/jan` backend.rs):
/// - Blackwell (sm_120): только cuda-13.x — сборка cuda-12.4 не имеет ядер;
/// - свежий драйвер CUDA 13+ (R580+): cuda-13.x — она содержит ядра sm_75..sm_120,
///   включая RTX 40xx, и является приоритетным выбором (как у Jan);
/// - остальные NVIDIA с драйвером CUDA 12: cuda-12.4.
pub fn required_cuda_gen(info: &GpuInfo) -> Option<CudaGen> {
    if !supports_cuda12(info) {
        return None;
    }
    // Blackwell (sm_120): сборка cuda-12.4 не имеет ядер для этих GPU.
    if info.compute_major >= 12 {
        return Some(CudaGen::Cuda13);
    }
    // Свежий драйвер CUDA 13+ → cuda-13.x (предпочтителен, как в Jan).
    // Если compute capability не определилась — полагаемся на драйвер.
    if info.cuda_major >= 13 {
        return Some(CudaGen::Cuda13);
    }
    // Драйвер CUDA 12.x: cuda-12.4 — самый совместимый вариант.
    Some(CudaGen::Cuda12)
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
    // compute capability: главный признак — есть ли в сборке движка ядра для GPU.
    let (compute_major, compute_minor) = match device.cuda_compute_capability() {
        Ok(cap) => (cap.major.max(0) as u32, cap.minor.max(0) as u32),
        Err(_) => (0, 0),
    };
    Ok(GpuInfo {
        has_nvidia: true,
        gpu_name: name,
        cuda_major: major,
        cuda_minor: minor,
        driver_version: driver,
        compute_major,
        compute_minor,
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
    let (compute_major, compute_minor) = query_compute_capability();
    Ok(GpuInfo {
        has_nvidia: true,
        gpu_name: "NVIDIA GPU (nvidia-smi)".to_string(),
        cuda_major,
        cuda_minor,
        driver_version: String::new(),
        compute_major,
        compute_minor,
    })
}

/// Compute capability через nvidia-smi --query-gpu=compute_cap (формат "12.0")
fn query_compute_capability() -> (u32, u32) {
    let mut cmd = Command::new("nvidia-smi");
    cmd.args(["--query-gpu=compute_cap", "--format=csv,noheader"]);
    #[cfg(target_os = "windows")]
    { use std::os::windows::process::CommandExt; cmd.creation_flags(0x08000000); }
    let output = match cmd.output() {
        Ok(o) if o.status.success() => o,
        _ => return (0, 0),
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let cap = line.trim();
        if cap.is_empty() {
            continue;
        }
        let parts: Vec<&str> = cap.split('.').collect();
        let major = parts
            .first()
            .and_then(|s| s.trim().parse::<u32>().ok())
            .unwrap_or(0);
        let minor = parts
            .get(1)
            .and_then(|s| s.trim().parse::<u32>().ok())
            .unwrap_or(0);
        return (major, minor);
    }
    (0, 0)
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
        let cc = if info.compute_major > 0 {
            format!(", compute {}.{}", info.compute_major, info.compute_minor)
        } else {
            String::new()
        };
        let variant = required_cuda_gen(info)
            .map(|g| format!("нужен вариант {}", g.label()))
            .unwrap_or_else(|| "".to_string());
        format!(
            "NVIDIA GPU: {} (драйвер CUDA {}.{}{}) — GPU-ускорение доступно ({}).",
            info.gpu_name, info.cuda_major, info.cuda_minor, cc, variant
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gpu(has_nvidia: bool, cuda_major: u32, compute_major: u32, compute_minor: u32) -> GpuInfo {
        GpuInfo {
            has_nvidia,
            gpu_name: "Test GPU".to_string(),
            cuda_major,
            cuda_minor: 0,
            driver_version: String::new(),
            compute_major,
            compute_minor,
        }
    }

    #[test]
    fn blackwell_requires_cuda13() {
        // RTX 5070 Ti: compute 12.0, драйвер CUDA 13.x
        let info = gpu(true, 13, 12, 0);
        assert_eq!(required_cuda_gen(&info), Some(CudaGen::Cuda13));
        assert!(supports_cuda12(&info));
    }

    #[test]
    fn rtx30_40_requires_cuda12() {
        // RTX 4060: compute 8.9, драйвер CUDA 12.x
        let info = gpu(true, 12, 8, 9);
        assert_eq!(required_cuda_gen(&info), Some(CudaGen::Cuda12));
        // RTX 3080: compute 8.6
        let info = gpu(true, 12, 8, 6);
        assert_eq!(required_cuda_gen(&info), Some(CudaGen::Cuda12));
        // GTX 1660: compute 7.5
        let info = gpu(true, 12, 7, 5);
        assert_eq!(required_cuda_gen(&info), Some(CudaGen::Cuda12));
    }

    #[test]
    fn rtx40_with_cuda13_driver_prefers_cuda13() {
        // RTX 4070 Ti Super: compute 8.9, но свежий драйвер CUDA 13 (R580+) —
        // сборка cuda-13.x содержит ядра sm_89, поэтому она предпочтительна (как в Jan).
        let info = gpu(true, 13, 8, 9);
        assert_eq!(required_cuda_gen(&info), Some(CudaGen::Cuda13));
        // RTX 3080 с драйвером CUDA 13 — тоже cuda-13.x
        let info = gpu(true, 13, 8, 6);
        assert_eq!(required_cuda_gen(&info), Some(CudaGen::Cuda13));
    }

    #[test]
    fn unknown_compute_cap_falls_back_to_cuda12() {
        let info = gpu(true, 12, 0, 0);
        assert_eq!(required_cuda_gen(&info), Some(CudaGen::Cuda12));
        // Compute не определился, но драйвер свежий CUDA 13 → cuda-13.x
        let info = gpu(true, 13, 0, 0);
        assert_eq!(required_cuda_gen(&info), Some(CudaGen::Cuda13));
    }

    #[test]
    fn no_gpu_or_old_driver_means_cpu() {
        assert_eq!(required_cuda_gen(&gpu(false, 0, 0, 0)), None);
        // Драйвер CUDA 11.x — слишком старый для движка CUDA 12
        assert_eq!(required_cuda_gen(&gpu(true, 11, 12, 0)), None);
        assert!(requires_driver_update(&gpu(true, 11, 12, 0)));
    }
}
