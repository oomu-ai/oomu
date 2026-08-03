use super::FullDiskAccessProbe;
use std::io::Read;

const PROBE_BYTES: usize = 16;

pub(crate) fn probe_full_disk_access() -> FullDiskAccessProbe {
    let Some(home) = dirs::home_dir() else {
        return FullDiskAccessProbe::Stale;
    };
    let target = home.join("Library/Application Support/com.apple.TCC/TCC.db");
    let mut file = match std::fs::File::open(target) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            return FullDiskAccessProbe::PermissionRequired;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return FullDiskAccessProbe::Unsupported;
        }
        Err(_) => return FullDiskAccessProbe::Stale,
    };
    let mut discarded = [0_u8; PROBE_BYTES];
    let result = file.read_exact(&mut discarded);
    discarded.fill(0);
    match result {
        Ok(()) => FullDiskAccessProbe::Allowed {
            bytes_read: PROBE_BYTES,
        },
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            FullDiskAccessProbe::PermissionRequired
        }
        Err(_) => FullDiskAccessProbe::Stale,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_reads_a_bounded_header_and_never_exports_it() {
        assert_eq!(PROBE_BYTES, 16);
        assert!(
            !format!("{:?}", FullDiskAccessProbe::Allowed { bytes_read: 16 })
                .contains("SQLite format")
        );
    }
}
