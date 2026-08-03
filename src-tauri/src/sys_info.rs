use serde::{Deserialize, Serialize};
use std::panic::{catch_unwind, AssertUnwindSafe};
use sysinfo::System;

pub const LOW_SPEC_LOCAL_CONTEXT_BUDGET: usize = 8_192;
pub const MID_SPEC_LOCAL_CONTEXT_BUDGET: usize = 16_384;
pub const HIGH_SPEC_LOCAL_CONTEXT_BUDGET: usize = 32_768;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HostHardwareTelemetry {
    pub cpu_arch: String,
    pub cpu_cores: usize,
    pub cpu_cores_available: bool,
    pub physical_ram_gb: u64,
    pub physical_ram_available: bool,
    pub os_name: String,
    pub metal_supported: bool,
    pub metal_probe_available: bool,
}

pub fn fetch_host_hardware_telemetry() -> HostHardwareTelemetry {
    let mut sys = System::new_all();
    sys.refresh_all();

    let cpu_cores = sys.cpus().len().max(
        std::thread::available_parallelism()
            .ok()
            .map_or(0, |value| value.get()),
    );
    let total_memory_bytes = sys.total_memory();
    let physical_ram_gb = bytes_to_gib(total_memory_bytes);
    let cpu_arch = std::env::consts::ARCH.to_string();
    let os_name = std::env::consts::OS.to_string();
    let metal_probe = probe_metal_backend();

    HostHardwareTelemetry {
        cpu_arch,
        cpu_cores,
        cpu_cores_available: cpu_cores > 0,
        physical_ram_gb,
        physical_ram_available: total_memory_bytes > 0,
        os_name,
        metal_supported: metal_probe.unwrap_or(false),
        metal_probe_available: metal_probe.is_some(),
    }
}

pub fn max_local_context_budget_for_physical_memory(physical_ram_gb: u64) -> usize {
    if physical_ram_gb < 16 {
        LOW_SPEC_LOCAL_CONTEXT_BUDGET
    } else if physical_ram_gb < 32 {
        MID_SPEC_LOCAL_CONTEXT_BUDGET
    } else {
        HIGH_SPEC_LOCAL_CONTEXT_BUDGET
    }
}

pub fn max_local_context_budget_for_telemetry(telemetry: &HostHardwareTelemetry) -> usize {
    if telemetry.physical_ram_available {
        max_local_context_budget_for_physical_memory(telemetry.physical_ram_gb)
    } else {
        LOW_SPEC_LOCAL_CONTEXT_BUDGET
    }
}

pub fn format_host_hardware_prompt_metadata(telemetry: &HostHardwareTelemetry) -> String {
    let cpu_cores = telemetry
        .cpu_cores_available
        .then(|| telemetry.cpu_cores.to_string())
        .unwrap_or_else(|| "unavailable".to_string());
    let physical_ram = telemetry
        .physical_ram_available
        .then(|| format!("{} GB", telemetry.physical_ram_gb))
        .unwrap_or_else(|| "unavailable".to_string());
    let metal_backend = telemetry
        .metal_probe_available
        .then(|| telemetry.metal_supported.to_string())
        .unwrap_or_else(|| "unavailable".to_string());
    format!(
        "\n[HOST HARDWARE METADATA]\n- OS: {}\n- CPU Architecture: {}\n- Logical CPU Cores: {}\n- Physical RAM: {}\n- Metal Backend Available: {}\n",
        telemetry.os_name,
        telemetry.cpu_arch,
        cpu_cores,
        physical_ram,
        metal_backend
    )
}

pub fn format_current_host_hardware_prompt_metadata() -> String {
    format_host_hardware_prompt_metadata(&fetch_host_hardware_telemetry())
}

fn bytes_to_gib(bytes: u64) -> u64 {
    bytes / 1024 / 1024 / 1024
}

fn probe_metal_backend() -> Option<bool> {
    if !cfg!(target_os = "macos") {
        return Some(false);
    }
    catch_unwind(AssertUnwindSafe(|| {
        let devices = llama_cpp_2::list_llama_ggml_backend_devices();
        crate::metal_backend::has_metal_device(&devices)
    }))
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_context_budget_scales_from_observed_physical_memory() {
        assert_eq!(max_local_context_budget_for_physical_memory(15), 8_192);
        assert_eq!(max_local_context_budget_for_physical_memory(16), 16_384);
        assert_eq!(max_local_context_budget_for_physical_memory(31), 16_384);
        assert_eq!(max_local_context_budget_for_physical_memory(32), 32_768);
    }

    #[test]
    fn host_hardware_prompt_metadata_names_runtime_capabilities() {
        let telemetry = HostHardwareTelemetry {
            cpu_arch: "aarch64".to_string(),
            cpu_cores: 12,
            cpu_cores_available: true,
            physical_ram_gb: 64,
            physical_ram_available: true,
            os_name: "macos".to_string(),
            metal_supported: true,
            metal_probe_available: true,
        };
        let metadata = format_host_hardware_prompt_metadata(&telemetry);

        assert!(metadata.contains("[HOST HARDWARE METADATA]"));
        assert!(metadata.contains("- OS: macos"));
        assert!(metadata.contains("- CPU Architecture: aarch64"));
        assert!(metadata.contains("- Logical CPU Cores: 12"));
        assert!(metadata.contains("- Physical RAM: 64 GB"));
        assert!(metadata.contains("- Metal Backend Available: true"));
        assert!(!metadata.to_ascii_lowercase().contains("estimated"));
    }

    #[test]
    fn host_hardware_telemetry_reports_nonempty_runtime_basics() {
        let telemetry = fetch_host_hardware_telemetry();

        assert!(!telemetry.cpu_arch.trim().is_empty());
        assert!(!telemetry.os_name.trim().is_empty());
        assert_eq!(telemetry.cpu_cores_available, telemetry.cpu_cores > 0);
        if !telemetry.physical_ram_available {
            assert!(format_host_hardware_prompt_metadata(&telemetry)
                .contains("Physical RAM: unavailable"));
        }
    }

    #[test]
    fn unavailable_probes_are_not_formatted_as_real_zero_measurements() {
        let telemetry = HostHardwareTelemetry {
            cpu_arch: "aarch64".to_string(),
            cpu_cores: 0,
            cpu_cores_available: false,
            physical_ram_gb: 0,
            physical_ram_available: false,
            os_name: "macos".to_string(),
            metal_supported: true,
            metal_probe_available: false,
        };
        let metadata = format_host_hardware_prompt_metadata(&telemetry);
        assert!(metadata.contains("Logical CPU Cores: unavailable"));
        assert!(metadata.contains("Physical RAM: unavailable"));
        assert!(metadata.contains("Metal Backend Available: unavailable"));
        assert_eq!(
            max_local_context_budget_for_telemetry(&telemetry),
            LOW_SPEC_LOCAL_CONTEXT_BUDGET
        );
    }
}
