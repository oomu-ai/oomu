use llama_cpp_2::{LlamaBackendDevice, LlamaBackendDeviceType};

pub(crate) fn is_metal_device(device: &LlamaBackendDevice) -> bool {
    matches!(
        device.device_type,
        LlamaBackendDeviceType::Gpu | LlamaBackendDeviceType::IntegratedGpu
    ) && metal_backend_rank(&device.backend).is_some()
}

pub(crate) fn has_metal_device(devices: &[LlamaBackendDevice]) -> bool {
    preferred_metal_device(devices).is_some()
}

pub(crate) fn preferred_metal_device(
    devices: &[LlamaBackendDevice],
) -> Option<&LlamaBackendDevice> {
    devices
        .iter()
        .filter(|device| is_metal_device(device))
        .min_by_key(|device| metal_backend_rank(&device.backend).unwrap_or(u8::MAX))
}

fn metal_backend_rank(backend: &str) -> Option<u8> {
    match backend.trim().to_ascii_lowercase().as_str() {
        "mtl" => Some(0),
        "metal" => Some(1),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(
        backend: &str,
        device_type: LlamaBackendDeviceType,
        index: usize,
    ) -> LlamaBackendDevice {
        LlamaBackendDevice {
            index,
            name: format!("device-{index}"),
            description: format!("{backend} device {index}"),
            backend: backend.to_string(),
            memory_total: index * 1_024,
            memory_free: index * 512,
            device_type,
        }
    }

    #[test]
    fn strict_metal_predicate_accepts_only_mtl_or_metal_gpu_devices() {
        for (backend, device_type, expected) in [
            ("MTL", LlamaBackendDeviceType::IntegratedGpu, true),
            ("Metal", LlamaBackendDeviceType::Gpu, true),
            ("Vulkan", LlamaBackendDeviceType::Gpu, false),
            ("RPC", LlamaBackendDeviceType::Gpu, false),
            ("CPU", LlamaBackendDeviceType::Cpu, false),
            ("NotMetal", LlamaBackendDeviceType::IntegratedGpu, false),
            ("MTL", LlamaBackendDeviceType::Cpu, false),
        ] {
            assert_eq!(
                is_metal_device(&device(backend, device_type, 1)),
                expected,
                "backend={backend:?} type={device_type:?}"
            );
        }
    }

    #[test]
    fn mtl_metadata_wins_regardless_of_device_enumeration_order() {
        let devices = vec![
            device("Vulkan", LlamaBackendDeviceType::Gpu, 1),
            device("Metal", LlamaBackendDeviceType::Gpu, 2),
            device("CPU", LlamaBackendDeviceType::Cpu, 3),
            device("MTL", LlamaBackendDeviceType::IntegratedGpu, 4),
        ];

        let selected = preferred_metal_device(&devices).expect("Metal accelerator exists");
        assert_eq!(selected.index, 4);
        assert_eq!(selected.backend, "MTL");
    }
}
