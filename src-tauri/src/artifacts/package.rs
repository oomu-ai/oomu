use crc32fast::Hasher;
use std::{collections::BTreeMap, fs, io::Write, path::Path};

pub(super) fn write_store_zip(
    path: &Path,
    entries: BTreeMap<String, Vec<u8>>,
) -> Result<(), String> {
    let mut file = fs::File::create(path).map_err(|error| error.to_string())?;
    let mut central = Vec::new();
    let mut offset = 0u32;
    for (name, data) in entries {
        let name_bytes = name.as_bytes();
        let mut hasher = Hasher::new();
        hasher.update(&data);
        let crc = hasher.finalize();
        let mut local = Vec::new();
        push_u32(&mut local, 0x04034b50);
        push_u16(&mut local, 20);
        push_u16(&mut local, 0x0800);
        push_u16(&mut local, 0);
        push_u16(&mut local, 0);
        push_u16(&mut local, 0);
        push_u32(&mut local, crc);
        push_u32(&mut local, data.len() as u32);
        push_u32(&mut local, data.len() as u32);
        push_u16(&mut local, name_bytes.len() as u16);
        push_u16(&mut local, 0);
        local.extend_from_slice(name_bytes);
        file.write_all(&local)
            .and_then(|_| file.write_all(&data))
            .map_err(|error| error.to_string())?;

        let mut entry = Vec::new();
        push_u32(&mut entry, 0x02014b50);
        push_u16(&mut entry, 20);
        push_u16(&mut entry, 20);
        push_u16(&mut entry, 0x0800);
        push_u16(&mut entry, 0);
        push_u16(&mut entry, 0);
        push_u16(&mut entry, 0);
        push_u32(&mut entry, crc);
        push_u32(&mut entry, data.len() as u32);
        push_u32(&mut entry, data.len() as u32);
        push_u16(&mut entry, name_bytes.len() as u16);
        push_u16(&mut entry, 0);
        push_u16(&mut entry, 0);
        push_u16(&mut entry, 0);
        push_u16(&mut entry, 0);
        push_u32(&mut entry, 0);
        push_u32(&mut entry, offset);
        entry.extend_from_slice(name_bytes);
        central.extend(entry);
        offset = offset.saturating_add((local.len() + data.len()) as u32);
    }

    let central_offset = offset;
    file.write_all(&central)
        .map_err(|error| error.to_string())?;
    let mut end = Vec::new();
    push_u32(&mut end, 0x06054b50);
    push_u16(&mut end, 0);
    push_u16(&mut end, 0);
    let count = count_central(&central);
    push_u16(&mut end, count);
    push_u16(&mut end, count);
    push_u32(&mut end, central.len() as u32);
    push_u32(&mut end, central_offset);
    push_u16(&mut end, 0);
    file.write_all(&end).map_err(|error| error.to_string())
}

fn count_central(bytes: &[u8]) -> u16 {
    bytes
        .windows(4)
        .filter(|window| *window == [0x50, 0x4b, 0x01, 0x02])
        .count() as u16
}

fn push_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes())
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes())
}
