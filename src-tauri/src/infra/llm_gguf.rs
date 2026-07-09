//! Утилиты чтения GGUF метаданных

fn read_gguf_header(path: &str) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let mut buffer = vec![0; 5 * 1024 * 1024];
    let bytes_read = file.read(&mut buffer).ok()?;
    let data = &buffer[..bytes_read];
    if data.len() < 24 || &data[0..4] != b"GGUF" { return None; }
    Some(data.to_vec())
}

fn skip_gguf_value(data: &[u8], mut offset: usize, val_type: u32) -> Option<usize> {
    match val_type {
        0 | 1 | 7 => Some(offset + 1),
        2 | 3 => Some(offset + 2),
        4 | 5 | 6 => Some(offset + 4),
        10 | 11 | 12 => Some(offset + 8),
        8 => {
            if offset + 8 > data.len() { return None; }
            let len = u64::from_le_bytes(data[offset..offset+8].try_into().unwrap()) as usize;
            Some(offset + 8 + len)
        },
        9 => {
            if offset + 4 > data.len() { return None; }
            let arr_type = u32::from_le_bytes(data[offset..offset+4].try_into().unwrap());
            offset += 4;
            if offset + 8 > data.len() { return None; }
            let arr_len = u64::from_le_bytes(data[offset..offset+8].try_into().unwrap()) as usize;
            offset += 8;
            for _ in 0..arr_len { offset = skip_gguf_value(data, offset, arr_type)?; }
            Some(offset)
        },
        _ => None
    }
}

fn find_gguf_value(path: &str, target_key: &str, expected_type: u32) -> Option<Vec<u8>> {
    let data = read_gguf_header(path)?;
    let kv_count = u64::from_le_bytes(data[16..24].try_into().unwrap());
    let mut offset = 24;
    for _ in 0..kv_count {
        if offset + 8 > data.len() { break; }
        let key_len = u64::from_le_bytes(data[offset..offset+8].try_into().unwrap()) as usize;
        offset += 8;
        if offset + key_len > data.len() { break; }
        let key = String::from_utf8_lossy(&data[offset..offset+key_len]);
        offset += key_len;
        if offset + 4 > data.len() { break; }
        let val_type = u32::from_le_bytes(data[offset..offset+4].try_into().unwrap());
        offset += 4;

        if key == target_key && val_type == expected_type {
            match val_type {
                4 | 6 => {
                    if offset + 4 > data.len() { break; }
                    return Some(data[offset..offset+4].to_vec());
                },
                8 => {
                    if offset + 8 > data.len() { break; }
                    let val_len = u64::from_le_bytes(data[offset..offset+8].try_into().unwrap()) as usize;
                    offset += 8;
                    if offset + val_len > data.len() { break; }
                    return Some(data[offset..offset+val_len].to_vec());
                },
                _ => return None,
            }
        } else {
            offset = skip_gguf_value(&data, offset, val_type)?;
        }
    }
    None
}

pub fn extract_string_from_gguf(path: &str, target_key: &str) -> Option<String> {
    String::from_utf8(find_gguf_value(path, target_key, 8)?).ok()
}

pub fn extract_f32_from_gguf(path: &str, target_key: &str) -> Option<f32> {
    Some(f32::from_le_bytes(find_gguf_value(path, target_key, 6)?.try_into().unwrap()))
}

pub fn extract_u32_from_gguf(path: &str, target_key: &str) -> Option<u32> {
    Some(u32::from_le_bytes(find_gguf_value(path, target_key, 4)?.try_into().unwrap()))
}