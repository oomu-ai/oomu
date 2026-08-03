use crc32fast::Hasher;
use flate2::read::DeflateDecoder;
use std::{collections::BTreeMap, io::Read};

const MAX_ENTRIES: usize = 4_096;
const MAX_UNCOMPRESSED_BYTES: u64 = 256 * 1024 * 1024;
const MAX_ENTRY_UNCOMPRESSED_BYTES: usize = 64 * 1024 * 1024;
const INITIAL_DEFLATE_ALLOCATION: usize = 64 * 1024;

pub(crate) fn read_zip(bytes: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let eocd = find_eocd(bytes)?;
    let entry_count = le_u16(bytes, eocd + 10)? as usize;
    let central_size = le_u32(bytes, eocd + 12)? as usize;
    let central_offset = le_u32(bytes, eocd + 16)? as usize;
    if entry_count == 0
        || entry_count > MAX_ENTRIES
        || central_offset.checked_add(central_size) != Some(eocd)
    {
        return Err("XLSX ZIP directory is empty, oversized, or truncated.".to_string());
    }
    let mut cursor = central_offset;
    let mut total_uncompressed = 0_u64;
    let mut entries = BTreeMap::new();
    for _ in 0..entry_count {
        if le_u32(bytes, cursor)? != 0x02014b50 {
            return Err("XLSX ZIP central directory is invalid.".to_string());
        }
        let flags = le_u16(bytes, cursor + 8)?;
        let method = le_u16(bytes, cursor + 10)?;
        let crc = le_u32(bytes, cursor + 16)?;
        let compressed = le_u32(bytes, cursor + 20)? as usize;
        let uncompressed = le_u32(bytes, cursor + 24)? as usize;
        let name_len = le_u16(bytes, cursor + 28)? as usize;
        let extra_len = le_u16(bytes, cursor + 30)? as usize;
        let comment_len = le_u16(bytes, cursor + 32)? as usize;
        let local_offset = le_u32(bytes, cursor + 42)? as usize;
        if flags & 0x0001 != 0 || !matches!(method, 0 | 8) {
            return Err("XLSX ZIP encryption or unsupported compression is forbidden.".to_string());
        }
        total_uncompressed = total_uncompressed
            .checked_add(uncompressed as u64)
            .ok_or_else(|| "XLSX ZIP expanded-size overflow.".to_string())?;
        if uncompressed > MAX_ENTRY_UNCOMPRESSED_BYTES
            || total_uncompressed > MAX_UNCOMPRESSED_BYTES
            || (compressed > 0 && uncompressed / compressed.max(1) > 500)
        {
            return Err("XLSX ZIP exceeds safe expansion limits.".to_string());
        }
        let name_start = cursor + 46;
        let name_end = name_start
            .checked_add(name_len)
            .ok_or_else(|| "XLSX ZIP filename overflow.".to_string())?;
        if name_end > bytes.len() {
            return Err("XLSX ZIP filename is truncated.".to_string());
        }
        let name = std::str::from_utf8(&bytes[name_start..name_end])
            .map_err(|_| "XLSX ZIP filenames must be UTF-8.".to_string())?
            .to_string();
        validate_entry_name(&name)?;
        if entries.contains_key(&name) {
            return Err(format!("XLSX ZIP entry {name} is duplicated."));
        }
        let data = read_local_entry(
            bytes,
            local_offset,
            name.as_bytes(),
            flags,
            method,
            compressed,
            uncompressed,
        )?;
        let mut hasher = Hasher::new();
        hasher.update(&data);
        if hasher.finalize() != crc {
            return Err(format!("XLSX ZIP entry {name} failed CRC verification."));
        }
        entries.insert(name, data);
        cursor = name_end
            .checked_add(extra_len + comment_len)
            .ok_or_else(|| "XLSX ZIP directory overflow.".to_string())?;
        if cursor > central_offset + central_size {
            return Err("XLSX ZIP central directory is truncated.".to_string());
        }
    }
    if cursor != central_offset + central_size {
        return Err("XLSX ZIP central directory contains trailing or missing bytes.".to_string());
    }
    Ok(entries)
}

fn find_eocd(bytes: &[u8]) -> Result<usize, String> {
    if bytes.len() < 22 {
        return Err("XLSX ZIP end record is missing.".to_string());
    }
    let start = bytes.len().saturating_sub(65_557);
    for index in (start..=bytes.len() - 4).rev() {
        if bytes[index..index + 4] == [0x50, 0x4b, 0x05, 0x06] {
            if index + 22 > bytes.len()
                || le_u16(bytes, index + 4)? != 0
                || le_u16(bytes, index + 6)? != 0
            {
                return Err("Multi-disk or truncated XLSX ZIP is forbidden.".to_string());
            }
            let comment_length = le_u16(bytes, index + 20)? as usize;
            if index.checked_add(22 + comment_length) != Some(bytes.len()) {
                continue;
            }
            return Ok(index);
        }
    }
    Err("XLSX ZIP end record is missing.".to_string())
}

fn read_local_entry(
    bytes: &[u8],
    offset: usize,
    central_name: &[u8],
    central_flags: u16,
    method: u16,
    compressed: usize,
    uncompressed: usize,
) -> Result<Vec<u8>, String> {
    if le_u32(bytes, offset)? != 0x04034b50 {
        return Err("XLSX ZIP local header is invalid.".to_string());
    }
    let local_flags = le_u16(bytes, offset + 6)?;
    let local_method = le_u16(bytes, offset + 8)?;
    if local_flags != central_flags || local_method != method {
        return Err("XLSX ZIP local and central compression metadata disagree.".to_string());
    }
    let name_len = le_u16(bytes, offset + 26)? as usize;
    let extra_len = le_u16(bytes, offset + 28)? as usize;
    let name_start = offset + 30;
    let name_end = name_start
        .checked_add(name_len)
        .ok_or_else(|| "XLSX ZIP local name overflow.".to_string())?;
    if name_end > bytes.len() || &bytes[name_start..name_end] != central_name {
        return Err("XLSX ZIP local and central filenames disagree.".to_string());
    }
    let data_start = name_end
        .checked_add(extra_len)
        .ok_or_else(|| "XLSX ZIP data offset overflow.".to_string())?;
    let data_end = data_start
        .checked_add(compressed)
        .ok_or_else(|| "XLSX ZIP data overflow.".to_string())?;
    if data_end > bytes.len() {
        return Err("XLSX ZIP entry is truncated.".to_string());
    }
    let source = &bytes[data_start..data_end];
    let data = if method == 0 {
        source.to_vec()
    } else {
        decode_deflate_bounded(source, uncompressed)?
    };
    if data.len() != uncompressed {
        return Err("XLSX ZIP entry expanded length is invalid.".to_string());
    }
    Ok(data)
}

fn decode_deflate_bounded(source: &[u8], expected: usize) -> Result<Vec<u8>, String> {
    if expected > MAX_ENTRY_UNCOMPRESSED_BYTES {
        return Err("XLSX ZIP entry exceeds the per-entry expansion limit.".to_string());
    }
    let limit = expected
        .checked_add(1)
        .ok_or_else(|| "XLSX ZIP expanded-size overflow.".to_string())? as u64;
    let decoder = DeflateDecoder::new(source);
    let mut bounded = decoder.take(limit);
    let mut output = Vec::with_capacity(expected.min(INITIAL_DEFLATE_ALLOCATION));
    bounded
        .read_to_end(&mut output)
        .map_err(|error| format!("XLSX ZIP deflate decode failed: {error}"))?;
    if output.len() != expected {
        return Err("XLSX ZIP entry expanded length is invalid.".to_string());
    }
    Ok(output)
}

pub(crate) fn write_store_zip(entries: &BTreeMap<String, Vec<u8>>) -> Result<Vec<u8>, String> {
    if entries.is_empty() || entries.len() > MAX_ENTRIES {
        return Err("XLSX ZIP entry count is invalid.".to_string());
    }
    let total = entries
        .values()
        .try_fold(0_u64, |sum, data| sum.checked_add(data.len() as u64))
        .ok_or_else(|| "XLSX ZIP size overflow.".to_string())?;
    if total > MAX_UNCOMPRESSED_BYTES {
        return Err("XLSX ZIP exceeds the bounded package size.".to_string());
    }
    let mut output = Vec::new();
    let mut directory = Vec::new();
    for (name, data) in entries {
        validate_entry_name(name)?;
        let name_bytes = name.as_bytes();
        let size =
            u32::try_from(data.len()).map_err(|_| "XLSX ZIP entry is too large.".to_string())?;
        let offset = u32::try_from(output.len())
            .map_err(|_| "XLSX ZIP exceeds classic ZIP bounds.".to_string())?;
        let mut hasher = Hasher::new();
        hasher.update(data);
        let crc = hasher.finalize();
        push_u32(&mut output, 0x04034b50);
        push_u16(&mut output, 20);
        push_u16(&mut output, 0x0800);
        push_u16(&mut output, 0);
        push_u16(&mut output, 0);
        push_u16(&mut output, 0x0021);
        push_u32(&mut output, crc);
        push_u32(&mut output, size);
        push_u32(&mut output, size);
        push_u16(&mut output, name_bytes.len() as u16);
        push_u16(&mut output, 0);
        output.extend_from_slice(name_bytes);
        output.extend_from_slice(data);

        push_u32(&mut directory, 0x02014b50);
        push_u16(&mut directory, 20);
        push_u16(&mut directory, 20);
        push_u16(&mut directory, 0x0800);
        push_u16(&mut directory, 0);
        push_u16(&mut directory, 0);
        push_u16(&mut directory, 0x0021);
        push_u32(&mut directory, crc);
        push_u32(&mut directory, size);
        push_u32(&mut directory, size);
        push_u16(&mut directory, name_bytes.len() as u16);
        push_u16(&mut directory, 0);
        push_u16(&mut directory, 0);
        push_u16(&mut directory, 0);
        push_u16(&mut directory, 0);
        push_u32(&mut directory, 0);
        push_u32(&mut directory, offset);
        directory.extend_from_slice(name_bytes);
    }
    let directory_offset = u32::try_from(output.len())
        .map_err(|_| "XLSX ZIP exceeds classic ZIP bounds.".to_string())?;
    let directory_size = u32::try_from(directory.len())
        .map_err(|_| "XLSX ZIP directory is too large.".to_string())?;
    output.extend_from_slice(&directory);
    push_u32(&mut output, 0x06054b50);
    push_u16(&mut output, 0);
    push_u16(&mut output, 0);
    push_u16(&mut output, entries.len() as u16);
    push_u16(&mut output, entries.len() as u16);
    push_u32(&mut output, directory_size);
    push_u32(&mut output, directory_offset);
    push_u16(&mut output, 0);
    Ok(output)
}

fn validate_entry_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.starts_with('/')
        || name.starts_with('\\')
        || name.contains("..")
        || name.contains('\\')
        || name.bytes().any(|value| value < 0x20)
    {
        Err(format!("XLSX ZIP entry path {name} is unsafe."))
    } else {
        Ok(())
    }
}

fn le_u16(bytes: &[u8], offset: usize) -> Result<u16, String> {
    let slice = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| "XLSX ZIP integer is truncated.".to_string())?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}

fn le_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let slice = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| "XLSX ZIP integer is truncated.".to_string())?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn push_u16(output: &mut Vec<u8>, value: u16) {
    output.extend_from_slice(&value.to_le_bytes());
}
fn push_u32(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{write::DeflateEncoder, Compression};
    use std::io::Write;

    #[test]
    fn deterministic_store_zip_round_trips_and_rejects_unsafe_paths() {
        let entries = BTreeMap::from([("xl/workbook.xml".to_string(), b"<workbook/>".to_vec())]);
        let first = write_store_zip(&entries).unwrap();
        assert_eq!(first, write_store_zip(&entries).unwrap());
        assert_eq!(read_zip(&first).unwrap(), entries);
        assert!(write_store_zip(&BTreeMap::from([("../bad".to_string(), vec![])])).is_err());
    }

    #[test]
    fn rejects_trailing_bytes_and_local_metadata_disagreement() {
        let entries = BTreeMap::from([("xl/workbook.xml".to_string(), b"<workbook/>".to_vec())]);
        let mut trailing = write_store_zip(&entries).unwrap();
        trailing.extend_from_slice(b"junk");
        assert!(read_zip(&trailing).is_err());

        let mut mismatched = write_store_zip(&entries).unwrap();
        mismatched[8] = 8;
        assert!(read_zip(&mismatched).unwrap_err().contains("disagree"));
    }

    #[test]
    fn deflate_expansion_stops_at_the_declared_length() {
        let payload = vec![b'x'; 2 * 1024 * 1024];
        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::best());
        encoder.write_all(&payload).unwrap();
        let compressed = encoder.finish().unwrap();
        let error = decode_deflate_bounded(&compressed, 16).unwrap_err();
        assert!(error.contains("expanded length"));
    }
}
