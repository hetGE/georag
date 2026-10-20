//! Backend probe and selection (handoff section 5.2).
//!
//! The probe lists every ggml device with its free and total memory, which
//! ggml reads per backend: Metal reports the recommended working set, CUDA
//! uses `cudaMemGetInfo`, Vulkan the device heap budget.

use std::fmt;
use std::str::FromStr;
use std::sync::OnceLock;

use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::{LlamaBackendDeviceType, LlamaCppError};
use serde::Serialize;

use crate::FLAVOUR;
use crate::error::EngineError;

/// A compute backend GeoRAG can run on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    Metal,
    Cuda,
    Vulkan,
    Cpu,
}

impl Backend {
    /// The name ggml uses for the backend registry entry.
    fn ggml_name(self) -> &'static str {
        match self {
            Backend::Metal => "Metal",
            Backend::Cuda => "CUDA",
            Backend::Vulkan => "Vulkan",
            Backend::Cpu => "CPU",
        }
    }

    /// ggml registers Metal as `MTL` (devices `MTL0`); older builds said `Metal`.
    fn from_ggml_name(name: &str) -> Option<Backend> {
        match name.to_ascii_lowercase().as_str() {
            "mtl" | "metal" => Some(Backend::Metal),
            "cuda" => Some(Backend::Cuda),
            "vulkan" => Some(Backend::Vulkan),
            "cpu" => Some(Backend::Cpu),
            _ => None,
        }
    }
}

impl fmt::Display for Backend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.ggml_name())
    }
}

/// `settings.engine.backend` or `--backend`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendPreference {
    Auto,
    Fixed(Backend),
}

impl FromStr for BackendPreference {
    type Err = EngineError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" | "" => Ok(BackendPreference::Auto),
            "metal" => Ok(BackendPreference::Fixed(Backend::Metal)),
            "cuda" => Ok(BackendPreference::Fixed(Backend::Cuda)),
            "vulkan" => Ok(BackendPreference::Fixed(Backend::Vulkan)),
            "cpu" => Ok(BackendPreference::Fixed(Backend::Cpu)),
            other => Err(EngineError::InvalidBackend(other.to_string())),
        }
    }
}

/// ggml's device class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceKind {
    Cpu,
    Accelerator,
    Gpu,
    IntegratedGpu,
    Unknown,
}

impl fmt::Display for DeviceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            DeviceKind::Cpu => "CPU",
            DeviceKind::Accelerator => "accelerator",
            DeviceKind::Gpu => "GPU",
            DeviceKind::IntegratedGpu => "integrated GPU",
            DeviceKind::Unknown => "unknown",
        })
    }
}

/// One compute device as ggml reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeviceInfo {
    /// ggml device name, for example `Metal`, `CUDA0`, `Vulkan0`, `CPU`.
    pub name: String,
    /// Hardware description, for example `Apple M4`.
    pub description: String,
    /// Backend registry name, for example `Metal`.
    pub backend: String,
    pub kind: DeviceKind,
    pub memory_total: u64,
    pub memory_free: u64,
}

/// The backend chosen for this run, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BackendInfo {
    pub flavour: &'static str,
    pub backend: Backend,
    /// One sentence explaining the choice, for `doctor` and the status bar.
    pub reason: String,
    /// The device the models will run on.
    pub device: DeviceInfo,
    /// Every device the probe found.
    pub all_devices: Vec<DeviceInfo>,
    /// Set when the choice is a fallback the user should know about.
    pub warning: Option<String>,
}

static BACKEND: OnceLock<Result<LlamaBackend, String>> = OnceLock::new();

/// Initialise llama.cpp once for the process and route its log lines into
/// `tracing`. Later calls are no-ops.
pub fn ensure_initialised() -> Result<(), EngineError> {
    let result = BACKEND.get_or_init(|| {
        llama_cpp_2::send_logs_to_tracing(llama_cpp_2::LogOptions::default());
        match LlamaBackend::init() {
            Ok(backend) => Ok(backend),
            Err(LlamaCppError::BackendAlreadyInitialized) => Ok(LlamaBackend {}),
            Err(e) => Err(e.to_string()),
        }
    });
    match result {
        Ok(_) => Ok(()),
        Err(message) => Err(EngineError::Init(message.clone())),
    }
}

/// List every ggml device with its memory figures.
pub fn probe_devices() -> Result<Vec<DeviceInfo>, EngineError> {
    ensure_initialised()?;
    let devices: Vec<DeviceInfo> = llama_cpp_2::list_llama_ggml_backend_devices()
        .into_iter()
        .map(|d| DeviceInfo {
            name: d.name,
            description: d.description,
            backend: d.backend,
            kind: match d.device_type {
                LlamaBackendDeviceType::Cpu => DeviceKind::Cpu,
                LlamaBackendDeviceType::Accelerator => DeviceKind::Accelerator,
                LlamaBackendDeviceType::Gpu => DeviceKind::Gpu,
                LlamaBackendDeviceType::IntegratedGpu => DeviceKind::IntegratedGpu,
                LlamaBackendDeviceType::Unknown => DeviceKind::Unknown,
            },
            memory_total: d.memory_total as u64,
            memory_free: d.memory_free as u64,
        })
        .collect();
    if devices.is_empty() {
        return Err(EngineError::NoDevices);
    }
    tracing::debug!(count = devices.len(), "probed ggml devices");
    Ok(devices)
}

/// Choose the backend following handoff section 5.2:
/// 1. the configured backend when not `auto`;
/// 2. Metal on Apple silicon;
/// 3. CUDA when this is the CUDA flavour and a CUDA device is present;
/// 4. Vulkan when this is the Vulkan flavour and a Vulkan device is present;
/// 5. CPU, with a warning.
pub fn select_backend(pref: BackendPreference) -> Result<BackendInfo, EngineError> {
    let devices = probe_devices()?;
    select_from(pref, devices)
}

fn select_from(
    pref: BackendPreference,
    devices: Vec<DeviceInfo>,
) -> Result<BackendInfo, EngineError> {
    let find = |backend: Backend| {
        devices
            .iter()
            .find(|d| Backend::from_ggml_name(&d.backend) == Some(backend))
            .cloned()
    };
    let cpu = find(Backend::Cpu)
        .or_else(|| devices.first().cloned())
        .ok_or(EngineError::NoDevices)?;

    let (backend, device, reason, warning) = match pref {
        BackendPreference::Fixed(requested) => {
            let device = find(requested).ok_or(EngineError::BackendUnavailable {
                requested,
                flavour: FLAVOUR,
            })?;
            (
                requested,
                device,
                format!("{requested} was set in settings or on the command line"),
                None,
            )
        }
        BackendPreference::Auto => {
            if let Some(d) = apple_silicon().then(|| find(Backend::Metal)).flatten() {
                (
                    Backend::Metal,
                    d,
                    "Apple silicon: Metal device present".to_string(),
                    None,
                )
            } else if let Some(d) = cfg!(feature = "cuda")
                .then(|| find(Backend::Cuda))
                .flatten()
            {
                (
                    Backend::Cuda,
                    d,
                    "CUDA flavour and a CUDA device is present".to_string(),
                    None,
                )
            } else if let Some(d) = cfg!(feature = "vulkan")
                .then(|| find(Backend::Vulkan))
                .flatten()
            {
                (
                    Backend::Vulkan,
                    d,
                    "Vulkan flavour and a Vulkan device is present".to_string(),
                    None,
                )
            } else {
                let reason = if cfg!(feature = "cuda") {
                    "CUDA flavour but no CUDA device was found".to_string()
                } else if cfg!(feature = "vulkan") {
                    "Vulkan flavour but no Vulkan device was found".to_string()
                } else if apple_silicon() {
                    "Apple silicon but no Metal device was found".to_string()
                } else {
                    "this flavour has no GPU backend".to_string()
                };
                (
                    Backend::Cpu,
                    cpu,
                    reason,
                    Some(
                        "running on the CPU: generation will be slow and large models will not fit"
                            .to_string(),
                    ),
                )
            }
        }
    };

    if let Some(w) = &warning {
        tracing::warn!("{w}");
    }
    Ok(BackendInfo {
        flavour: FLAVOUR,
        backend,
        reason,
        device,
        all_devices: devices,
        warning,
    })
}

fn apple_silicon() -> bool {
    cfg!(all(target_os = "macos", target_arch = "aarch64"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(name: &str, backend: &str, kind: DeviceKind) -> DeviceInfo {
        DeviceInfo {
            name: name.into(),
            description: format!("{name} desc"),
            backend: backend.into(),
            kind,
            memory_total: 16 << 30,
            memory_free: 12 << 30,
        }
    }

    #[test]
    fn preference_parses_case_insensitively() {
        assert_eq!(
            "Auto".parse::<BackendPreference>().unwrap(),
            BackendPreference::Auto
        );
        assert_eq!(
            "CPU".parse::<BackendPreference>().unwrap(),
            BackendPreference::Fixed(Backend::Cpu)
        );
        assert!("opencl".parse::<BackendPreference>().is_err());
    }

    #[test]
    fn fixed_backend_must_exist() {
        let devices = vec![dev("CPU", "CPU", DeviceKind::Cpu)];
        let err = select_from(BackendPreference::Fixed(Backend::Cuda), devices).unwrap_err();
        assert!(matches!(
            err,
            EngineError::BackendUnavailable {
                requested: Backend::Cuda,
                ..
            }
        ));
    }

    #[test]
    fn fixed_cpu_always_works_when_present() {
        let devices = vec![
            dev("Metal", "Metal", DeviceKind::Gpu),
            dev("CPU", "CPU", DeviceKind::Cpu),
        ];
        let info = select_from(BackendPreference::Fixed(Backend::Cpu), devices).unwrap();
        assert_eq!(info.backend, Backend::Cpu);
        assert_eq!(info.device.name, "CPU");
    }

    #[test]
    fn auto_falls_back_to_cpu_with_warning() {
        let devices = vec![dev("CPU", "CPU", DeviceKind::Cpu)];
        let info = select_from(BackendPreference::Auto, devices).unwrap();
        assert_eq!(info.backend, Backend::Cpu);
        assert!(info.warning.is_some());
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    #[test]
    fn auto_prefers_metal_on_apple_silicon() {
        let devices = vec![
            dev("CPU", "CPU", DeviceKind::Cpu),
            dev("MTL0", "MTL", DeviceKind::Gpu),
        ];
        let info = select_from(BackendPreference::Auto, devices).unwrap();
        assert_eq!(info.backend, Backend::Metal);
        assert_eq!(info.device.name, "MTL0");
        assert!(info.warning.is_none());
    }

    #[test]
    fn ggml_backend_names_map_to_backends() {
        assert_eq!(Backend::from_ggml_name("MTL"), Some(Backend::Metal));
        assert_eq!(Backend::from_ggml_name("Metal"), Some(Backend::Metal));
        assert_eq!(Backend::from_ggml_name("CUDA"), Some(Backend::Cuda));
        assert_eq!(Backend::from_ggml_name("Vulkan"), Some(Backend::Vulkan));
        assert_eq!(Backend::from_ggml_name("BLAS"), None);
    }
}
