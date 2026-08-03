use std::io::{Cursor, Read as _, Write as _};

pub(super) fn legacy_xls_bytes(title: &str, content: &str) -> Result<Vec<u8>, String> {
    let sheet_name = bounded_biff_text(title, 31, "Sheet1");
    let cell_text = bounded_biff_text(content, 32_767, "");
    let mut globals = Vec::new();
    push_biff_record(&mut globals, 0x0809, &biff_bof(0x0005));
    push_biff_record(&mut globals, 0x0042, &0x04B0_u16.to_le_bytes());
    push_biff_record(
        &mut globals,
        0x003D,
        &[
            0x00, 0x00, 0x00, 0x00, 0xCF, 0x3F, 0x4E, 0x2A, 0x38, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x01, 0x00, 0x58, 0x02,
        ],
    );
    push_biff_record(&mut globals, 0x0031, &biff_font());
    push_biff_record(&mut globals, 0x00E0, &biff_xf());
    let boundsheet_record_start = globals.len();
    let mut boundsheet = vec![0_u8; 6];
    push_short_unicode(&mut boundsheet, &sheet_name)?;
    push_biff_record(&mut globals, 0x0085, &boundsheet);
    push_biff_record(&mut globals, 0x000A, &[]);
    let sheet_offset =
        u32::try_from(globals.len()).map_err(|_| "Legacy workbook is too large.".to_string())?;
    globals[boundsheet_record_start + 4..boundsheet_record_start + 8]
        .copy_from_slice(&sheet_offset.to_le_bytes());

    let mut sheet = Vec::new();
    push_biff_record(&mut sheet, 0x0809, &biff_bof(0x0010));
    push_biff_record(&mut sheet, 0x0081, &0x04C1_u16.to_le_bytes());
    let mut dimensions = Vec::with_capacity(14);
    dimensions.extend_from_slice(&0_u32.to_le_bytes());
    dimensions.extend_from_slice(&1_u32.to_le_bytes());
    dimensions.extend_from_slice(&0_u16.to_le_bytes());
    dimensions.extend_from_slice(&1_u16.to_le_bytes());
    dimensions.extend_from_slice(&0_u16.to_le_bytes());
    push_biff_record(&mut sheet, 0x0200, &dimensions);
    let mut label = Vec::new();
    label.extend_from_slice(&0_u16.to_le_bytes());
    label.extend_from_slice(&0_u16.to_le_bytes());
    label.extend_from_slice(&0_u16.to_le_bytes());
    push_long_unicode(&mut label, &cell_text)?;
    push_biff_record(&mut sheet, 0x0204, &label);
    push_biff_record(
        &mut sheet,
        0x023E,
        &[
            0xB6, 0x06, 0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00,
        ],
    );
    push_biff_record(&mut sheet, 0x000A, &[]);
    globals.extend_from_slice(&sheet);

    let cursor = Cursor::new(Vec::new());
    let mut compound = cfb::CompoundFile::create_with_version(cfb::Version::V3, cursor)
        .map_err(|error| format!("Legacy workbook container could not be created: {error}"))?;
    compound
        .create_stream("/Workbook")
        .and_then(|mut stream| stream.write_all(&globals))
        .map_err(|error| format!("Legacy workbook stream could not be created: {error}"))?;
    let bytes = compound.into_inner().into_inner();
    verify_legacy_xls_bytes(&bytes, &cell_text)?;
    Ok(bytes)
}

fn push_biff_record(output: &mut Vec<u8>, record_type: u16, data: &[u8]) {
    output.extend_from_slice(&record_type.to_le_bytes());
    output.extend_from_slice(&(data.len() as u16).to_le_bytes());
    output.extend_from_slice(data);
}

fn biff_bof(document_type: u16) -> Vec<u8> {
    let mut data = Vec::with_capacity(16);
    data.extend_from_slice(&0x0600_u16.to_le_bytes());
    data.extend_from_slice(&document_type.to_le_bytes());
    data.extend_from_slice(&0x0DBB_u16.to_le_bytes());
    data.extend_from_slice(&0x07CC_u16.to_le_bytes());
    data.extend_from_slice(&0x00000041_u32.to_le_bytes());
    data.extend_from_slice(&0x00000006_u32.to_le_bytes());
    data
}

fn biff_font() -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(&200_u16.to_le_bytes());
    data.extend_from_slice(&0_u16.to_le_bytes());
    data.extend_from_slice(&0x7FFF_u16.to_le_bytes());
    data.extend_from_slice(&400_u16.to_le_bytes());
    data.extend_from_slice(&0_u16.to_le_bytes());
    data.extend_from_slice(&[0, 0, 0, 0]);
    data.push(5);
    data.push(0);
    data.extend_from_slice(b"Arial");
    data
}

fn biff_xf() -> [u8; 20] {
    let mut data = [0_u8; 20];
    data[4..6].copy_from_slice(&0_u16.to_le_bytes());
    data
}

fn push_short_unicode(output: &mut Vec<u8>, value: &str) -> Result<(), String> {
    let units = value.encode_utf16().collect::<Vec<_>>();
    output.push(u8::try_from(units.len()).map_err(|_| "Sheet name is too long.".to_string())?);
    output.push(1);
    for unit in units {
        output.extend_from_slice(&unit.to_le_bytes());
    }
    Ok(())
}

fn push_long_unicode(output: &mut Vec<u8>, value: &str) -> Result<(), String> {
    let units = value.encode_utf16().collect::<Vec<_>>();
    output.extend_from_slice(
        &u16::try_from(units.len())
            .map_err(|_| "Cell text is too long.".to_string())?
            .to_le_bytes(),
    );
    output.push(1);
    for unit in units {
        output.extend_from_slice(&unit.to_le_bytes());
    }
    Ok(())
}

fn bounded_biff_text(value: &str, max_units: usize, fallback: &str) -> String {
    let candidate = value.trim();
    let candidate = if candidate.is_empty() {
        fallback
    } else {
        candidate
    };
    let mut used = 0;
    candidate
        .chars()
        .take_while(|character| {
            let next = used + character.len_utf16();
            if next > max_units {
                false
            } else {
                used = next;
                true
            }
        })
        .collect()
}

pub(super) fn verify_legacy_xls_bytes(bytes: &[u8], expected_text: &str) -> Result<(), String> {
    let mut compound = cfb::CompoundFile::open(Cursor::new(bytes))
        .map_err(|error| format!("Legacy workbook container check failed: {error}"))?;
    let mut workbook = Vec::new();
    compound
        .open_stream("/Workbook")
        .and_then(|mut stream| stream.read_to_end(&mut workbook))
        .map_err(|error| format!("Legacy workbook stream check failed: {error}"))?;
    let mut offset = 0;
    let mut bof_count = 0;
    let mut boundsheet_offset = None;
    let mut observed_text = None;
    while offset + 4 <= workbook.len() {
        let kind = u16::from_le_bytes([workbook[offset], workbook[offset + 1]]);
        let length = u16::from_le_bytes([workbook[offset + 2], workbook[offset + 3]]) as usize;
        let data_start = offset + 4;
        let data_end = data_start + length;
        if data_end > workbook.len() {
            return Err("Legacy workbook record length check failed.".to_string());
        }
        let data = &workbook[data_start..data_end];
        match kind {
            0x0809 => bof_count += 1,
            0x0085 if data.len() >= 4 => {
                boundsheet_offset = Some(u32::from_le_bytes([data[0], data[1], data[2], data[3]]));
            }
            0x0204 if data.len() >= 9 => {
                let count = u16::from_le_bytes([data[6], data[7]]) as usize;
                let high_byte = data[8] & 1 == 1;
                let string_bytes = &data[9..];
                observed_text = if high_byte && string_bytes.len() >= count * 2 {
                    Some(String::from_utf16_lossy(
                        &(0..count)
                            .map(|index| {
                                u16::from_le_bytes([
                                    string_bytes[index * 2],
                                    string_bytes[index * 2 + 1],
                                ])
                            })
                            .collect::<Vec<_>>(),
                    ))
                } else if !high_byte && string_bytes.len() >= count {
                    Some(String::from_utf8_lossy(&string_bytes[..count]).to_string())
                } else {
                    None
                };
            }
            _ => {}
        }
        offset = data_end;
    }
    if bof_count != 2
        || boundsheet_offset.is_none_or(|value| value as usize >= workbook.len())
        || observed_text.as_deref() != Some(expected_text)
    {
        return Err("Legacy workbook structural checks did not pass.".to_string());
    }
    Ok(())
}
