use flate2::read::DeflateDecoder;
use std::{
    collections::HashSet,
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

const LOCAL_FILE_HEADER_SIGNATURE: u32 = 0x0403_4b50;
const CENTRAL_DIRECTORY_SIGNATURE: u32 = 0x0201_4b50;
const END_OF_CENTRAL_DIRECTORY_SIGNATURE: u32 = 0x0605_4b50;
pub(super) const MAX_MOD_ARCHIVE_SIZE: u64 = 200 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 256;

#[derive(Debug)]
pub(super) struct ArchiveEntry {
    pub(super) name: String,
    pub(super) bytes: Vec<u8>,
}

#[derive(Debug)]
struct CentralDirectoryEntry {
    name: String,
    compression_method: u16,
    flags: u16,
    crc32: u32,
    compressed_size: usize,
    uncompressed_size: usize,
    local_header_offset: usize,
}

pub(super) fn parse_mod_archive(bytes: &[u8]) -> Result<Vec<ArchiveEntry>, String> {
    let central_directory_offset = find_central_directory(bytes)?;
    let central_entries = read_central_directory(bytes, central_directory_offset)?;
    if central_entries.is_empty() {
        return Err("Mod archive does not contain any files.".to_string());
    }
    if central_entries.len() > MAX_ARCHIVE_ENTRIES {
        return Err(format!(
            "Mod archive contains too many files. Maximum supported entries: {MAX_ARCHIVE_ENTRIES}."
        ));
    }
    ensure_unique_archive_names(central_entries.iter().map(|entry| entry.name.as_str()))?;

    let mut total_uncompressed = 0_u64;
    let mut entries = Vec::new();
    for central_entry in central_entries {
        if central_entry.name.ends_with('/') {
            continue;
        }
        total_uncompressed =
            total_uncompressed.saturating_add(central_entry.uncompressed_size as u64);
        if total_uncompressed > MAX_MOD_ARCHIVE_SIZE {
            return Err(format!(
                "Expanded mod package is too large. Maximum supported size is {}.",
                format_package_size(MAX_MOD_ARCHIVE_SIZE)
            ));
        }
        let bytes = read_archive_file(bytes, &central_entry)?;
        entries.push(ArchiveEntry {
            name: central_entry.name,
            bytes,
        });
    }
    Ok(entries)
}

fn ensure_unique_archive_names<'a>(names: impl IntoIterator<Item = &'a str>) -> Result<(), String> {
    let mut seen = HashSet::new();
    for name in names {
        let collision_key = name.to_lowercase();
        if !seen.insert(collision_key) {
            return Err("Mod archive contains duplicate file paths.".to_string());
        }
    }
    Ok(())
}

fn find_central_directory(bytes: &[u8]) -> Result<usize, String> {
    let search_start = bytes.len().saturating_sub(65_557);
    for offset in (search_start..bytes.len().saturating_sub(3)).rev() {
        if read_u32_at(bytes, offset)? == END_OF_CENTRAL_DIRECTORY_SIGNATURE {
            let central_directory_offset = read_u32_at(bytes, offset + 16)? as usize;
            if central_directory_offset >= bytes.len() {
                return Err("Mod archive central directory is invalid.".to_string());
            }
            return Ok(central_directory_offset);
        }
    }
    Err("Mod package is not a valid ZIP archive.".to_string())
}

fn read_central_directory(
    bytes: &[u8],
    mut offset: usize,
) -> Result<Vec<CentralDirectoryEntry>, String> {
    let mut entries = Vec::new();
    while offset + 4 <= bytes.len() {
        let signature = read_u32_at(bytes, offset)?;
        if signature == END_OF_CENTRAL_DIRECTORY_SIGNATURE {
            break;
        }
        if signature != CENTRAL_DIRECTORY_SIGNATURE {
            return Err("Mod archive central directory is corrupt.".to_string());
        }

        let flags = read_u16_at(bytes, offset + 8)?;
        if flags & 0x01 != 0 {
            return Err("Encrypted mod archives are not supported.".to_string());
        }
        let compression_method = read_u16_at(bytes, offset + 10)?;
        let crc32 = read_u32_at(bytes, offset + 16)?;
        let compressed_size = read_u32_at(bytes, offset + 20)?;
        let uncompressed_size = read_u32_at(bytes, offset + 24)?;
        let name_len = read_u16_at(bytes, offset + 28)? as usize;
        let extra_len = read_u16_at(bytes, offset + 30)? as usize;
        let comment_len = read_u16_at(bytes, offset + 32)? as usize;
        let local_header_offset = read_u32_at(bytes, offset + 42)?;
        if compressed_size == u32::MAX
            || uncompressed_size == u32::MAX
            || local_header_offset == u32::MAX
        {
            return Err("ZIP64 mod archives are not supported yet.".to_string());
        }

        let name_start = offset + 46;
        let name_end = name_start
            .checked_add(name_len)
            .ok_or_else(|| "Mod archive filename is too large.".to_string())?;
        if name_end > bytes.len() {
            return Err("Mod archive central directory filename is truncated.".to_string());
        }
        let raw_name = std::str::from_utf8(&bytes[name_start..name_end])
            .map_err(|error| format!("Mod archive contains a non-UTF-8 filename: {error}"))?;
        let is_directory = raw_name.ends_with('/');
        let name = normalize_archive_name(raw_name)?;
        let name = if is_directory {
            format!("{name}/")
        } else {
            name
        };
        entries.push(CentralDirectoryEntry {
            name,
            compression_method,
            flags,
            crc32,
            compressed_size: compressed_size as usize,
            uncompressed_size: uncompressed_size as usize,
            local_header_offset: local_header_offset as usize,
        });

        offset = name_end
            .checked_add(extra_len)
            .and_then(|value| value.checked_add(comment_len))
            .ok_or_else(|| "Mod archive central directory offset overflowed.".to_string())?;
    }
    Ok(entries)
}

fn read_archive_file(bytes: &[u8], entry: &CentralDirectoryEntry) -> Result<Vec<u8>, String> {
    let offset = entry.local_header_offset;
    if read_u32_at(bytes, offset)? != LOCAL_FILE_HEADER_SIGNATURE {
        return Err(format!("Local header for {} is corrupt.", entry.name));
    }
    let local_flags = read_u16_at(bytes, offset + 6)?;
    if local_flags & 0x01 != 0 || entry.flags & 0x01 != 0 {
        return Err("Encrypted mod archives are not supported.".to_string());
    }
    let name_len = read_u16_at(bytes, offset + 26)? as usize;
    let extra_len = read_u16_at(bytes, offset + 28)? as usize;
    let name_start = offset
        .checked_add(30)
        .ok_or_else(|| "Mod archive filename offset overflowed.".to_string())?;
    let name_end = name_start
        .checked_add(name_len)
        .ok_or_else(|| "Mod archive filename is too large.".to_string())?;
    let raw_local_name = std::str::from_utf8(
        bytes
            .get(name_start..name_end)
            .ok_or_else(|| "Mod archive local filename is truncated.".to_string())?,
    )
    .map_err(|error| format!("Mod archive contains a non-UTF-8 filename: {error}"))?;
    let local_is_directory = raw_local_name.ends_with('/');
    let local_name = normalize_archive_name(raw_local_name)?;
    let local_name = if local_is_directory {
        format!("{local_name}/")
    } else {
        local_name
    };
    if local_name != entry.name {
        return Err("Mod archive local and central filenames do not match.".to_string());
    }
    let data_start = name_end
        .checked_add(extra_len)
        .ok_or_else(|| "Mod archive data offset overflowed.".to_string())?;
    let data_end = data_start
        .checked_add(entry.compressed_size)
        .ok_or_else(|| "Mod archive data size overflowed.".to_string())?;
    if data_end > bytes.len() {
        return Err(format!(
            "File {} is truncated in the mod archive.",
            entry.name
        ));
    }
    let compressed = &bytes[data_start..data_end];
    let mut file_bytes = match entry.compression_method {
        0 => compressed.to_vec(),
        8 => {
            let decoder = DeflateDecoder::new(compressed);
            let mut decoded = Vec::with_capacity(entry.uncompressed_size);
            decoder
                .take(entry.uncompressed_size as u64 + 1)
                .read_to_end(&mut decoded)
                .map_err(|error| format!("Unable to decompress {}: {error}", entry.name))?;
            decoded
        }
        method => {
            return Err(format!(
                "Unsupported compression method {method} in {}.",
                entry.name
            ));
        }
    };
    if file_bytes.len() != entry.uncompressed_size {
        return Err(format!(
            "File {} expanded to an unexpected size.",
            entry.name
        ));
    }
    if crc32fast::hash(&file_bytes) != entry.crc32 {
        return Err(format!("File {} failed checksum verification.", entry.name));
    }
    file_bytes.shrink_to_fit();
    Ok(file_bytes)
}

pub(super) fn extract_entries_to(
    entries: &[ArchiveEntry],
    destination: &Path,
) -> Result<(), String> {
    for entry in entries {
        let output_path = destination.join(relative_archive_path(&entry.name)?);
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "Unable to create mod package directory {}: {error}",
                    parent.display()
                )
            })?;
        }
        fs::write(&output_path, &entry.bytes).map_err(|error| {
            format!(
                "Unable to write extracted mod package file {}: {error}",
                output_path.display()
            )
        })?;
    }
    Ok(())
}

pub(super) fn normalize_archive_name(raw_name: &str) -> Result<String, String> {
    let name = raw_name.replace('\\', "/");
    if name.trim().is_empty() || name.starts_with('/') || !name.is_ascii() {
        return Err("Mod archive contains an invalid file path.".to_string());
    }
    let clean = relative_archive_path(&name)?;
    Ok(clean.to_string_lossy().replace('\\', "/"))
}

pub(super) fn relative_archive_path(name: &str) -> Result<PathBuf, String> {
    let path = Path::new(name);
    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => clean.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(
                    "Mod archive contains a path that escapes the package root.".to_string()
                );
            }
        }
    }
    if clean.as_os_str().is_empty() {
        return Err("Mod archive contains an empty file path.".to_string());
    }
    Ok(clean)
}

fn format_package_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut size = bytes as f64;
    let mut unit_index = 0;
    while size >= 1024.0 && unit_index < UNITS.len() - 1 {
        size /= 1024.0;
        unit_index += 1;
    }
    if unit_index == 0 {
        format!("{} {}", bytes, UNITS[unit_index])
    } else {
        format!("{size:.1} {}", UNITS[unit_index])
    }
}

fn read_u16_at(bytes: &[u8], offset: usize) -> Result<u16, String> {
    let end = offset
        .checked_add(2)
        .ok_or_else(|| "ZIP offset overflowed.".to_string())?;
    let slice = bytes
        .get(offset..end)
        .ok_or_else(|| "ZIP structure is truncated.".to_string())?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_u32_at(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let end = offset
        .checked_add(4)
        .ok_or_else(|| "ZIP offset overflowed.".to_string())?;
    let slice = bytes
        .get(offset..end)
        .ok_or_else(|| "ZIP structure is truncated.".to_string())?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{write::DeflateEncoder, Compression};
    use std::io::Write;

    #[test]
    fn duplicate_and_case_colliding_paths_are_rejected() {
        assert!(ensure_unique_archive_names(["code/main.js", "code/main.js"]).is_err());
        assert!(ensure_unique_archive_names(["Code/Main.js", "code/main.js"]).is_err());
        assert!(ensure_unique_archive_names(["code/a.js", "code/b.js"]).is_ok());
    }

    #[test]
    fn local_header_filename_must_match_central_directory() {
        let local_name = b"other.js";
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&LOCAL_FILE_HEADER_SIGNATURE.to_le_bytes());
        bytes.extend_from_slice(&[0; 22]);
        bytes.extend_from_slice(&(local_name.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes.extend_from_slice(local_name);
        let entry = CentralDirectoryEntry {
            name: "index.js".to_string(),
            compression_method: 0,
            flags: 0,
            crc32: 0,
            compressed_size: 0,
            uncompressed_size: 0,
            local_header_offset: 0,
        };
        assert!(read_archive_file(&bytes, &entry)
            .expect_err("ambiguous local filename is rejected")
            .contains("do not match"));
    }

    #[test]
    fn deflate_payload_cannot_expand_past_declared_size() {
        let name = b"index.js";
        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&vec![b'x'; 1024]).unwrap();
        let compressed = encoder.finish().unwrap();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&LOCAL_FILE_HEADER_SIGNATURE.to_le_bytes());
        bytes.extend_from_slice(&[0; 22]);
        bytes.extend_from_slice(&(name.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes.extend_from_slice(name);
        bytes.extend_from_slice(&compressed);
        let entry = CentralDirectoryEntry {
            name: "index.js".to_string(),
            compression_method: 8,
            flags: 0,
            crc32: 0,
            compressed_size: compressed.len(),
            uncompressed_size: 1,
            local_header_offset: 0,
        };
        assert!(read_archive_file(&bytes, &entry)
            .expect_err("declared size bounds decompression")
            .contains("unexpected size"));
    }
}
