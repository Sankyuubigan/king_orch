//! Утилиты чтения GGUF метаданных и валидации GGUF-файлов

use std::io::{BufReader, Read, Seek, SeekFrom};

fn read_gguf_header(path: &str) -> Option<Vec<u8>> {
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

// ============================================================
// Валидация целостности GGUF-файла
//
// Проверяем файл ДО запуска llama-server, чтобы вместо хвоста лога
// (check_tensor_dims: tensor 'blk.N.*' not found) пользователь увидел
// понятную ошибку. Набор проверок повторяет логику llama.cpp (gguf_init /
// llama-model-loader): сигнатура, версия, согласованность block_count с
// фактическими тензорами blk.N, обязательный тензор token_embd.weight,
// монотонность смещений и выход данных за конец файла.
// ============================================================

struct GgufReader<'a> {
    file: &'a mut BufReader<std::fs::File>,
    file_size: u64,
    offset: u64,
}

impl<'a> GgufReader<'a> {
    fn new(file: &'a mut BufReader<std::fs::File>, file_size: u64) -> Self {
        Self { file, file_size, offset: 0 }
    }

    fn need(&mut self, n: u64) -> Result<(), String> {
        if self.offset + n > self.file_size {
            return Err("GGUF-файл повреждён: данные обрезаны или заголовок неверен.".to_string());
        }
        Ok(())
    }

    fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), String> {
        self.need(buf.len() as u64)?;
        self.file.read_exact(buf).map_err(|e| format!("Не удалось прочитать GGUF-файл: {}", e))?;
        self.offset += buf.len() as u64;
        Ok(())
    }

    fn read_u32(&mut self) -> Result<u32, String> {
        let mut b = [0u8; 4];
        self.read_exact(&mut b)?;
        Ok(u32::from_le_bytes(b))
    }

    fn read_u64(&mut self) -> Result<u64, String> {
        let mut b = [0u8; 8];
        self.read_exact(&mut b)?;
        Ok(u64::from_le_bytes(b))
    }

    fn read_string(&mut self) -> Result<String, String> {
        let len = self.read_u64()? as usize;
        self.need(len as u64)?;
        let mut b = vec![0u8; len];
        self.read_exact(&mut b)?;
        Ok(String::from_utf8_lossy(&b).to_string())
    }

    /// Пропуск скалярного значения GGUF (без хранения).
    fn skip_scalar(&mut self, val_type: u32) -> Result<(), String> {
        match val_type {
            0 | 1 | 7 => { let mut b = [0u8; 1]; self.read_exact(&mut b) } // uint8/int8/bool
            2 | 3 => { let mut b = [0u8; 2]; self.read_exact(&mut b) }     // uint16/int16
            4 | 5 | 6 => { let mut b = [0u8; 4]; self.read_exact(&mut b) } // uint32/int32/float32
            8 => { let _ = self.read_string()?; Ok(()) }                    // string
            10 | 11 | 12 => { let mut b = [0u8; 8]; self.read_exact(&mut b) } // uint64/int64/float64
            _ => Err(format!("GGUF-файл повреждён: неизвестный тип значения метаданных {}", val_type)),
        }
    }

    /// Пропуск значения GGUF любого типа (включая массивы).
    fn skip_value(&mut self, val_type: u32) -> Result<(), String> {
        if val_type == 9 {
            let elem_type = self.read_u32()?;
            let count = self.read_u64()?;
            for _ in 0..count {
                self.skip_value(elem_type)?;
            }
            return Ok(());
        }
        self.skip_scalar(val_type)
    }
}

/// Значения метаданных, которые нам нужны для валидации.
enum KvVal {
    U32(u32),
    Str(String),
}

fn read_kv_value(reader: &mut GgufReader, val_type: u32) -> Result<Option<KvVal>, String> {
    match val_type {
        4 => Ok(Some(KvVal::U32(reader.read_u32()?))),
        8 => Ok(Some(KvVal::Str(reader.read_string()?))),
        _ => { reader.skip_value(val_type)?; Ok(None) }
    }
}

/// Индекс блока из имени тензора вида `blk.31.attn_norm.weight`.
fn parse_blk_index(name: &str) -> Option<u32> {
    let rest = name.strip_prefix("blk.")?;
    let num: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if num.is_empty() { return None; }
    num.parse().ok()
}

/// (размер блока в байтах, число элементов в блоке) для GGML-типов.
/// Неизвестные типы → None (для таких тензоров границы данных не проверяем).
fn ggml_type_layout(t: u32) -> Option<(u64, u64)> {
    let (size, block) = match t {
        0 => (4, 1),      // F32
        1 => (2, 1),      // F16
        2 => (18, 32),    // Q4_0
        3 => (20, 32),    // Q4_1
        4 => (9, 16),     // Q4_2 (legacy)
        5 => (11, 16),    // Q4_3 (legacy)
        6 => (22, 32),    // Q5_0
        7 => (24, 32),    // Q5_1
        8 => (34, 32),    // Q8_0
        9 => (36, 32),    // Q8_1
        10 => (84, 256),  // Q2_K
        11 => (110, 256), // Q3_K
        12 => (144, 256), // Q4_K
        13 => (176, 256), // Q5_K
        14 => (210, 256), // Q6_K
        15 => (292, 256), // Q8_K
        16 => (36, 256),  // IQ2_XXS
        17 => (60, 256),  // IQ2_XS
        18 => (48, 256),  // IQ3_XXS
        19 => (32, 256),  // IQ1_S
        20 => (18, 32),   // IQ4_NL
        21 => (88, 256),  // IQ3_S
        22 => (110, 256), // IQ3_M
        23 => (68, 256),  // IQ2_S
        24 => (92, 256),  // IQ2_M
        25 => (116, 256), // IQ4_XS
        26 => (1, 1),     // I8
        27 => (2, 1),     // I16
        28 => (4, 1),     // I32
        29 => (8, 1),     // I64
        30 => (8, 1),     // F64
        31 => (52, 256),  // IQ1_M
        32 => (200, 256), // IQ4_BS
        33 => (34, 256),  // TQ1_0
        34 => (66, 256),  // TQ2_0
        35 => (18, 32),   // IQ4_NL_4_4
        36 => (18, 32),   // IQ4_NL_4_8
        37 => (18, 32),   // IQ4_NL_8_8
        _ => return None,
    };
    Some((size, block))
}

fn align_up(offset: u64, alignment: u64) -> Result<u64, String> {
    if alignment == 0 { return Ok(offset); }
    let rem = offset % alignment;
    if rem == 0 { return Ok(offset); }
    offset.checked_add(alignment - rem)
        .ok_or_else(|| "GGUF-файл повреждён: переполнение выравнивания.".to_string())
}

/// Результат разбора секции тензоров GGUF.
struct TensorSection {
    metas: Vec<TensorMeta>,
    blk_indices: std::collections::HashSet<u32>,
    has_token_embd: bool,
}

/// Метаданные тензора из tensor-info секции.
struct TensorMeta {
    name: String,
    offset: u64,
    nbytes: Option<u64>,
}

/// Считывает и проверяет заголовок GGUF.
fn read_header(p: &mut GgufReader) -> Result<(u64, u64), String> {
    let mut magic = [0u8; 4];
    p.read_exact(&mut magic)?;
    if &magic != b"GGUF" {
        return Err("Файл не является GGUF-моделью (отсутствует сигнатура GGUF).".to_string());
    }
    let version = p.read_u32()?;
    if !(1..=3).contains(&version) {
        return Err(format!("GGUF-файл имеет неподдерживаемую версию {} (поддерживаются 1-3).", version));
    }
    let tensor_count = p.read_u64()?;
    let kv_count = p.read_u64()?;
    if tensor_count == 0 || tensor_count > 10_000_000 {
        return Err("Заголовок GGUF повреждён: неверное число тензоров.".to_string());
    }
    if kv_count > 10_000_000 {
        return Err("Заголовок GGUF повреждён: неверное число записей метаданных.".to_string());
    }
    Ok((tensor_count, kv_count))
}

/// Читает метаданные (KV-пары), извлекая только нужное для проверок.
fn read_metadata(p: &mut GgufReader, kv_count: u64) -> Result<(Option<String>, u64, Option<u32>), String> {
    let mut architecture: Option<String> = None;
    let mut alignment: u64 = 32;
    let mut block_count: Option<u32> = None;
    for _ in 0..kv_count {
        let key = p.read_string()?;
        let val_type = p.read_u32()?;
        let val = read_kv_value(p, val_type)?;
        if let Some(v) = val {
            match (key.as_str(), v) {
                ("general.architecture", KvVal::Str(s)) => architecture = Some(s),
                ("general.alignment", KvVal::U32(a)) if a > 0 => alignment = a as u64,
                (k, KvVal::U32(v)) => {
                    if let Some(arch) = &architecture {
                        if k == format!("{}.block_count", arch).as_str() {
                            block_count = Some(v);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    Ok((architecture, alignment, block_count))
}

/// Считывает и проверяет tensor-info секцию.
///
/// По спецификации GGUF секция идёт сразу после метаданных (без выравнивания);
/// выравнивается только начало секции данных.
fn read_tensor_infos(p: &mut GgufReader, tensor_count: u64) -> Result<TensorSection, String> {
    let info_start = p.offset;
    p.file.seek(SeekFrom::Start(info_start)).map_err(|e| format!("Не удалось прочитать GGUF-файл: {}", e))?;
    p.offset = info_start;

    let mut section = TensorSection {
        metas: Vec::with_capacity(tensor_count as usize),
        blk_indices: std::collections::HashSet::new(),
        has_token_embd: false,
    };
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for _ in 0..tensor_count {
        let name = p.read_string()?;
        if !seen.insert(name.clone()) {
            return Err(format!("Заголовок GGUF повреждён: дубликат тензора «{}».", name));
        }
        if name == "token_embd.weight" { section.has_token_embd = true; }
        if let Some(idx) = parse_blk_index(&name) { section.blk_indices.insert(idx); }

        let n_dims = p.read_u32()?;
        if !(1..=4).contains(&n_dims) {
            return Err(format!("Заголовок GGUF повреждён: тензор «{}» имеет недопустимое число измерений ({}).", name, n_dims));
        }
        let mut ne: u64 = 1;
        for _ in 0..n_dims {
            // По спецификации GGUF каждое измерение хранится как uint64 (8 байт).
            let d = p.read_u64()? as u64;
            if d == 0 {
                return Err(format!("Заголовок GGUF повреждён: тензор «{}» имеет нулевое измерение.", name));
            }
            ne = ne.checked_mul(d)
                .ok_or_else(|| format!("Заголовок GGUF повреждён: переполнение размеров тензора «{}».", name))?;
        }
        let ggml_type = p.read_u32()?;
        let offset = p.read_u64()?;

        let nbytes = ggml_type_layout(ggml_type)
            .map(|(size, block)| ne.checked_add(block - 1).map(|v| v / block).and_then(|v| v.checked_mul(size)))
            .flatten();

        section.metas.push(TensorMeta { name, offset, nbytes });
    }
    Ok(section)
}

/// Проверяет, что число объявленных слоёв совпадает с фактическими `blk.N`.
///
/// Ключевая проверка: llama.cpp итерирует блоки от 0 до block_count-1 и падает,
/// если тензора `blk.N.*` нет (случай битой конвертации qwen3.8-9b).
fn check_block_count(block_count: Option<u32>, blk_indices: &std::collections::HashSet<u32>) -> Result<(), String> {
    if let Some(bc) = block_count {
        if let Some(&max_idx) = blk_indices.iter().max() {
            let found = max_idx as u64 + 1;
            if found != bc as u64 {
                return Err(format!(
                    "Файл модели повреждён: заголовок GGUF объявляет {} слоёв, но в файле найдено только {} блоков тензоров (blk.0..blk.{}). \
                     Файл сконвертирован или скачан некорректно — скачайте модель заново или выберите другую квантовку.",
                    bc, found, max_idx
                ));
            }
        }
    }
    Ok(())
}

/// Проверяет монотонность смещений данных и выход за конец файла.
fn check_offsets(metas: &[TensorMeta], data_len: u64) -> Result<(), String> {
    let mut prev_offset: Option<u64> = None;
    for m in metas {
        if let Some(prev) = prev_offset {
            if m.offset < prev {
                return Err(format!(
                    "Файл модели повреждён: нарушен порядок данных тензоров (offset {} после {}).",
                    m.offset, prev
                ));
            }
        }
        prev_offset = Some(m.offset);

        if let Some(nbytes) = m.nbytes {
            if let Some(end) = m.offset.checked_add(nbytes) {
                if end > data_len {
                    return Err(format!(
                        "Файл модели повреждён: данные тензора «{}» выходят за конец файла (файл обрезан или повреждён).",
                        m.name
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Проверка целостности GGUF-файла модели.
///
/// Возвращает `Err` с понятным русским сообщением, если файл повреждён или
/// сконвертирован некорректно (например, заголовок объявляет слоёв больше,
/// чем реально есть тензоров `blk.N` — llama.cpp падает в этом случае с
/// `check_tensor_dims: tensor 'blk.N.*' not found`).
pub fn validate_gguf(path: &str) -> Result<(), String> {
    let file = std::fs::File::open(path)
        .map_err(|e| format!("Не удалось открыть файл модели «{}»: {}", path, e))?;
    let file_size = file.metadata().map(|m| m.len()).unwrap_or(0);
    let mut buf = BufReader::new(file);
    let mut p = GgufReader::new(&mut buf, file_size);

    let (tensor_count, kv_count) = read_header(&mut p)?;
    let (_, alignment, block_count) = read_metadata(&mut p, kv_count)?;
    let section = read_tensor_infos(&mut p, tensor_count)?;

    check_block_count(block_count, &section.blk_indices)?;

    if !section.has_token_embd {
        return Err("Файл модели повреждён: не найден обязательный тензор token_embd.weight.".to_string());
    }

    let info_end = p.offset;
    let data_start = align_up(info_end, alignment)?;
    let data_len = file_size.saturating_sub(data_start);

    check_offsets(&section.metas, data_len)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct TempFile { path: PathBuf }

    impl TempFile {
        fn new(name: &str, bytes: &[u8]) -> Self {
            let path = std::env::temp_dir().join(format!("ko_gguf_test_{}_{}.bin", std::process::id(), name));
            let _ = std::fs::remove_file(&path);
            std::fs::write(&path, bytes).expect("запись temp-файла GGUF");
            TempFile { path }
        }
        fn truncate(&self, len: u64) {
            let f = std::fs::OpenOptions::new().write(true).open(&self.path).expect("открыть temp-файл");
            f.set_len(len).expect("урезать temp-файл");
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) { let _ = std::fs::remove_file(&self.path); }
    }

    struct Buf { v: Vec<u8> }

    impl Buf {
        fn new() -> Self { Buf { v: Vec::new() } }
        fn u32(&mut self, x: u32) { self.v.extend_from_slice(&x.to_le_bytes()); }
        fn u64(&mut self, x: u64) { self.v.extend_from_slice(&x.to_le_bytes()); }
        fn string(&mut self, s: &str) { self.u64(s.len() as u64); self.v.extend_from_slice(s.as_bytes()); }
    }

    fn kv_u32(key: &str, val: u32) -> Vec<u8> {
        let mut b = Buf::new();
        b.string(key); b.u32(4); b.u32(val);
        b.v
    }

    fn kv_string(key: &str, val: &str) -> Vec<u8> {
        let mut b = Buf::new();
        b.string(key); b.u32(8); b.string(val);
        b.v
    }

    fn align(v: u64, a: u64) -> u64 { if a == 0 { v } else { (v + a - 1) / a * a } }

    struct TensorSpec { name: &'static str, offset: u64, nbytes: u64 }

    fn ts(name: &'static str, offset: u64) -> TensorSpec { TensorSpec { name, offset, nbytes: 16 } }

    /// Минимальный валидный GGUF: F32-тензоры по 4 элемента, arch "testarch".
    /// `data_extra` — сколько лишних байт данных дописать в конец.
    fn build_gguf(declared_blocks: u32, tensors: &[TensorSpec]) -> Vec<u8> {
        let mut kv = Vec::new();
        kv.extend_from_slice(&kv_string("general.architecture", "testarch"));
        kv.extend_from_slice(&kv_u32("testarch.block_count", declared_blocks));
        kv.extend_from_slice(&kv_u32("general.alignment", 32));

        let mut header = Buf::new();
        header.v.extend_from_slice(b"GGUF");
        header.u32(3);
        header.u64(tensors.len() as u64);
        header.u64(3);
        header.v.extend_from_slice(&kv);

        // Tensor-info идёт сразу после метаданных (по спецификации GGUF).
        let info_start = header.v.len() as u64;

        let mut info = Vec::new();
        for t in tensors {
            let mut b = Buf::new();
            b.string(t.name);
            b.u32(1);   // n_dims
            b.u64(4);   // 4 элемента (u64 по спецификации)
            b.u32(0);   // F32
            b.u64(t.offset);
            info.extend_from_slice(&b.v);
        }
        // Выравнивается только начало секции данных.
        let data_start = align(info_start + info.len() as u64, 32);
        let data_len = tensors.iter().map(|t| t.offset + t.nbytes).max().unwrap_or(0) as usize;

        let mut out = header.v;
        out.extend_from_slice(&info);
        out.resize(data_start as usize + data_len, 0);
        out
    }

    fn valid_tensors() -> Vec<TensorSpec> {
        vec![ts("blk.0.attn_norm.weight", 0), ts("blk.1.attn_norm.weight", 16), ts("token_embd.weight", 32)]
    }

    #[test]
    fn valid_gguf_passes() {
        let file = TempFile::new("valid", &build_gguf(2, &valid_tensors()));
        assert_eq!(validate_gguf(file.path.to_str().unwrap()), Ok(()));
    }

    #[test]
    fn block_count_mismatch_fails() {
        // Заголовок объявляет 3 слоя, а тензоры есть только для blk.0..blk.1 —
        // ровно баг qwen3.8-9b (объявлено 33, реально 32 блока).
        let file = TempFile::new("mismatch", &build_gguf(3, &valid_tensors()));
        let err = validate_gguf(file.path.to_str().unwrap()).unwrap_err();
        assert!(err.contains("объявляет 3 слоёв"), "ошибка: {}", err);
        assert!(err.contains("только 2 блоков"), "ошибка: {}", err);
        assert!(err.contains("скачайте модель заново"), "ошибка: {}", err);
    }

    #[test]
    fn missing_token_embd_fails() {
        let tensors = vec![ts("blk.0.attn_norm.weight", 0), ts("blk.1.attn_norm.weight", 16)];
        let file = TempFile::new("no_embd", &build_gguf(2, &tensors));
        let err = validate_gguf(file.path.to_str().unwrap()).unwrap_err();
        assert!(err.contains("token_embd.weight"), "ошибка: {}", err);
    }

    #[test]
    fn truncated_file_fails() {
        let bytes = build_gguf(2, &valid_tensors());
        let file = TempFile::new("trunc", &bytes);
        // Урезаем файл внутри данных третьего тензора (token_embd на offset 32):
        // data_len = 3 * 16 = 48 байт, оставляем 39.
        file.truncate((bytes.len() - 9) as u64);
        let err = validate_gguf(file.path.to_str().unwrap()).unwrap_err();
        assert!(err.contains("выходят за конец файла"), "ошибка: {}", err);
    }

    #[test]
    fn non_monotonic_offsets_fail() {
        let tensors = vec![
            ts("token_embd.weight", 32),
            ts("blk.0.attn_norm.weight", 0),
            ts("blk.1.attn_norm.weight", 16),
        ];
        let file = TempFile::new("order", &build_gguf(2, &tensors));
        let err = validate_gguf(file.path.to_str().unwrap()).unwrap_err();
        assert!(err.contains("нарушен порядок данных"), "ошибка: {}", err);
    }

    #[test]
    fn not_gguf_fails() {
        let file = TempFile::new("not_gguf", b"This is definitely not a GGUF model file, just plain text. 0123456789");
        let err = validate_gguf(file.path.to_str().unwrap()).unwrap_err();
        assert!(err.contains("не является GGUF-моделью"), "ошибка: {}", err);
    }
}
