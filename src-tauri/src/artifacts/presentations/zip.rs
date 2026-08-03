use crc32fast::Hasher;
use flate2::read::DeflateDecoder;
use std::{collections::BTreeMap, io::Read};

const MAX_ENTRIES: usize = 8_192;
const MAX_UNCOMPRESSED_BYTES: u64 = 256 * 1024 * 1024;
const MAX_ENTRY_BYTES: usize = 64 * 1024 * 1024;

pub(crate) fn read_zip(bytes: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let eocd = find_eocd(bytes)?;
    let count = le_u16(bytes, eocd + 10)? as usize;
    let directory_size = le_u32(bytes, eocd + 12)? as usize;
    let directory_offset = le_u32(bytes, eocd + 16)? as usize;
    if count == 0
        || count > MAX_ENTRIES
        || directory_offset.checked_add(directory_size) != Some(eocd)
    {
        return Err("PPTX ZIP directory is empty, oversized, or truncated.".to_string());
    }
    let mut cursor = directory_offset;
    let mut total = 0_u64;
    let mut entries = BTreeMap::new();
    for _ in 0..count {
        if le_u32(bytes, cursor)? != 0x02014b50 {
            return Err("PPTX ZIP central directory is invalid.".to_string());
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
            return Err("PPTX ZIP encryption or unsupported compression is forbidden.".to_string());
        }
        total = total
            .checked_add(uncompressed as u64)
            .ok_or_else(|| "PPTX ZIP expanded-size overflow.".to_string())?;
        if uncompressed > MAX_ENTRY_BYTES
            || total > MAX_UNCOMPRESSED_BYTES
            || (compressed > 0 && uncompressed / compressed.max(1) > 500)
        {
            return Err("PPTX ZIP exceeds safe expansion limits.".to_string());
        }
        let name_start = cursor + 46;
        let name_end = name_start
            .checked_add(name_len)
            .ok_or_else(|| "PPTX ZIP filename overflow.".to_string())?;
        let name = std::str::from_utf8(
            bytes
                .get(name_start..name_end)
                .ok_or_else(|| "PPTX ZIP filename is truncated.".to_string())?,
        )
        .map_err(|_| "PPTX ZIP filenames must be UTF-8.".to_string())?
        .to_string();
        validate_name(&name)?;
        if entries.contains_key(&name) {
            return Err(format!("PPTX ZIP entry {name} is duplicated."));
        }
        let data = read_local(
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
            return Err(format!("PPTX ZIP entry {name} failed CRC verification."));
        }
        entries.insert(name, data);
        cursor = name_end
            .checked_add(extra_len + comment_len)
            .ok_or_else(|| "PPTX ZIP directory overflow.".to_string())?;
    }
    if cursor != directory_offset + directory_size {
        return Err("PPTX ZIP directory is truncated.".to_string());
    }
    Ok(entries)
}

fn find_eocd(bytes: &[u8]) -> Result<usize, String> {
    if bytes.len() < 22 {
        return Err("PPTX ZIP end record is missing.".to_string());
    }
    let start = bytes.len().saturating_sub(65_557);
    for index in (start..=bytes.len() - 4).rev() {
        if bytes[index..index + 4] == [0x50, 0x4b, 0x05, 0x06] {
            if index + 22 > bytes.len()
                || le_u16(bytes, index + 4)? != 0
                || le_u16(bytes, index + 6)? != 0
            {
                return Err("Multi-disk or truncated PPTX ZIP is forbidden.".to_string());
            }
            let comment = le_u16(bytes, index + 20)? as usize;
            if index.checked_add(22 + comment) == Some(bytes.len()) {
                return Ok(index);
            }
        }
    }
    Err("PPTX ZIP end record is missing.".to_string())
}

fn read_local(
    bytes: &[u8],
    offset: usize,
    expected_name: &[u8],
    flags: u16,
    method: u16,
    compressed: usize,
    uncompressed: usize,
) -> Result<Vec<u8>, String> {
    if le_u32(bytes, offset)? != 0x04034b50
        || le_u16(bytes, offset + 6)? != flags
        || le_u16(bytes, offset + 8)? != method
    {
        return Err("PPTX ZIP local metadata is invalid.".to_string());
    }
    let name_len = le_u16(bytes, offset + 26)? as usize;
    let extra_len = le_u16(bytes, offset + 28)? as usize;
    let name_start = offset + 30;
    let name_end = name_start
        .checked_add(name_len)
        .ok_or_else(|| "PPTX ZIP local name overflow.".to_string())?;
    if bytes.get(name_start..name_end) != Some(expected_name) {
        return Err("PPTX ZIP local and central filenames disagree.".to_string());
    }
    let data_start = name_end
        .checked_add(extra_len)
        .ok_or_else(|| "PPTX ZIP data offset overflow.".to_string())?;
    let data_end = data_start
        .checked_add(compressed)
        .ok_or_else(|| "PPTX ZIP data overflow.".to_string())?;
    let source = bytes
        .get(data_start..data_end)
        .ok_or_else(|| "PPTX ZIP entry is truncated.".to_string())?;
    let data = if method == 0 {
        source.to_vec()
    } else {
        let limit = uncompressed
            .checked_add(1)
            .ok_or_else(|| "PPTX ZIP expanded-size overflow.".to_string())?
            as u64;
        let mut decoder = DeflateDecoder::new(source).take(limit);
        let mut output = Vec::with_capacity(uncompressed.min(64 * 1024));
        decoder
            .read_to_end(&mut output)
            .map_err(|error| format!("PPTX ZIP deflate decode failed: {error}"))?;
        output
    };
    if data.len() != uncompressed {
        return Err("PPTX ZIP entry expanded length is invalid.".to_string());
    }
    Ok(data)
}

pub(crate) fn write_store_zip(entries: &BTreeMap<String, Vec<u8>>) -> Result<Vec<u8>, String> {
    if entries.is_empty() || entries.len() > MAX_ENTRIES {
        return Err("PPTX ZIP entry count is invalid.".to_string());
    }
    let total = entries
        .values()
        .try_fold(0_u64, |sum, data| sum.checked_add(data.len() as u64))
        .ok_or_else(|| "PPTX ZIP size overflow.".to_string())?;
    if total > MAX_UNCOMPRESSED_BYTES {
        return Err("PPTX ZIP exceeds the bounded package size.".to_string());
    }
    let mut output = Vec::new();
    let mut directory = Vec::new();
    for (name, data) in entries {
        validate_name(name)?;
        let name_bytes = name.as_bytes();
        let size = u32::try_from(data.len()).map_err(|_| "PPTX entry is too large.".to_string())?;
        let offset = u32::try_from(output.len())
            .map_err(|_| "PPTX package exceeds classic ZIP bounds.".to_string())?;
        let mut hasher = Hasher::new();
        hasher.update(data);
        let crc = hasher.finalize();
        push_local(&mut output, name_bytes, size, crc);
        output.extend_from_slice(name_bytes);
        output.extend_from_slice(data);
        push_directory(&mut directory, name_bytes, size, crc, offset);
    }
    let offset = u32::try_from(output.len()).map_err(|_| "PPTX ZIP overflow.".to_string())?;
    let size = u32::try_from(directory.len()).map_err(|_| "PPTX ZIP overflow.".to_string())?;
    output.extend_from_slice(&directory);
    push_u32(&mut output, 0x06054b50);
    push_u16(&mut output, 0);
    push_u16(&mut output, 0);
    push_u16(&mut output, entries.len() as u16);
    push_u16(&mut output, entries.len() as u16);
    push_u32(&mut output, size);
    push_u32(&mut output, offset);
    push_u16(&mut output, 0);
    Ok(output)
}

fn push_local(out: &mut Vec<u8>, name: &[u8], size: u32, crc: u32) {
    push_u32(out, 0x04034b50);
    push_u16(out, 20);
    push_u16(out, 0x0800);
    push_u16(out, 0);
    push_u16(out, 0);
    push_u16(out, 0x0021);
    push_u32(out, crc);
    push_u32(out, size);
    push_u32(out, size);
    push_u16(out, name.len() as u16);
    push_u16(out, 0);
}

fn push_directory(out: &mut Vec<u8>, name: &[u8], size: u32, crc: u32, offset: u32) {
    push_u32(out, 0x02014b50);
    push_u16(out, 20);
    push_u16(out, 20);
    push_u16(out, 0x0800);
    push_u16(out, 0);
    push_u16(out, 0);
    push_u16(out, 0x0021);
    push_u32(out, crc);
    push_u32(out, size);
    push_u32(out, size);
    push_u16(out, name.len() as u16);
    for _ in 0..4 {
        push_u16(out, 0);
    }
    push_u32(out, 0);
    push_u32(out, offset);
    out.extend_from_slice(name);
}

fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.starts_with('/')
        || name.starts_with('\\')
        || name.contains("..")
        || name.contains('\\')
        || name.bytes().any(|byte| byte < 0x20)
    {
        Err(format!("Unsafe PPTX ZIP entry path {name}."))
    } else {
        Ok(())
    }
}

fn le_u16(bytes: &[u8], offset: usize) -> Result<u16, String> {
    let value = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| "PPTX ZIP integer is truncated.".to_string())?;
    Ok(u16::from_le_bytes([value[0], value[1]]))
}

fn le_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| "PPTX ZIP integer is truncated.".to_string())?;
    Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

fn push_u16(output: &mut Vec<u8>, value: u16) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_le_bytes());
}
