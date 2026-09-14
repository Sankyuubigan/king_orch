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

/// Размер скалярного GGUF-значения в байтах (для расчёта длины массива).
/// Строки внутри массивов не поддерживаются (в GGUF-гиперпараметрах не бывают).
fn gguf_scalar_size(val_type: u32) -> Option<usize> {
    match val_type {
        0 | 1 | 7 => Some(1),
        2 | 3 => Some(2),
        4 | 5 | 6 => Some(4),
        10 | 11 | 12 => Some(8),
        _ => None,
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

        if key == target_key && (val_type == expected_type || expected_type == 9) {
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
                9 => {
                    // Массив: возвращаем сырые байты (тип элемента + длина + элементы) —
                    // парсинг в extract_i64_array_from_gguf. Едем только по размеру
                    // элементов, чтобы не читать (и не скипать) их здесь повторно.
                    if offset + 12 > data.len() { break; }
                    let elem_type = u32::from_le_bytes(data[offset..offset+4].try_into().unwrap());
                    let count = u64::from_le_bytes(data[offset+4..offset+12].try_into().unwrap()) as usize;
                    let elem_size = gguf_scalar_size(elem_type)?;
                    let total = 12 + count.checked_mul(elem_size)?;
                    if offset + total > data.len() { break; }
                    return Some(data[offset..offset+total].to_vec());
                },
                _ => return None,
            }
        } else {
            offset = skip_gguf_value(&data, offset, val_type)?;
        }
    }
    None
}

/// Читает значение GGUF типа ARRAY по ключу и возвращает его как `Vec<i64>`.
/// Поддерживаются числовые типы элементов (u8..i64, bool, f32); строковый
/// массив не имеет смысла для гиперпараметров — возвращает None. Формат хранения
/// (u32 head_count_kv и т.п.) часто скалярен: ищем ровно массив (тип 9).
pub fn extract_i64_array_from_gguf(path: &str, target_key: &str) -> Option<Vec<i64>> {
    let data = find_gguf_value(path, target_key, 9)?;
    if data.len() < 12 { return None; }
    let elem_type = u32::from_le_bytes(data[0..4].try_into().unwrap());
    let count = u64::from_le_bytes(data[4..12].try_into().unwrap()) as usize;
    let mut out = Vec::with_capacity(count);
    let mut o = 12;
    for _ in 0..count {
        let v: i64 = match elem_type {
            0 => { let b = data.get(o)?; o += 1; *b as i64 },                 // uint8
            1 => { let b = data.get(o)?; o += 1; *b as i8 as i64 },           // int8
            2 => { let b = data.get(o..o+2)?.try_into().ok()?; o += 2; u16::from_le_bytes(b) as i64 },
            3 => { let b = data.get(o..o+2)?.try_into().ok()?; o += 2; i16::from_le_bytes(b) as i64 },
            4 => { let b = data.get(o..o+4)?.try_into().ok()?; o += 4; u32::from_le_bytes(b) as i64 },
            5 => { let b = data.get(o..o+4)?.try_into().ok()?; o += 4; i32::from_le_bytes(b) as i64 },
            6 => { let b = data.get(o..o+4)?.try_into().ok()?; o += 4; f32::from_le_bytes(b) as i64 },
            7 => { let b = data.get(o)?; o += 1; (*b != 0) as i64 },          // bool
            10 => { let b = data.get(o..o+8)?.try_into().ok()?; o += 8; u64::from_le_bytes(b) as i64 },
            11 | 12 => { let b = data.get(o..o+8)?.try_into().ok()?; o += 8; i64::from_le_bytes(b) },
            _ => return None,
        };
        out.push(v);
    }
    Some(out)
}

/// Читает i64-массив GGUF с учётом префикса архитектуры (`<arch>.<suffix>`,
/// затем legacy `llama.<suffix>`). Скалярный ключ (обычная GQA-модель) → None.
pub fn extract_i64_array_with_arch(path: &str, arch: Option<&str>, suffix: &str) -> Option<Vec<i64>> {
    if let Some(arch) = arch.filter(|a| !a.is_empty()) {
        if let Some(v) = extract_i64_array_from_gguf(path, &format!("{}.{}", arch, suffix)) {
            return Some(v);
        }
    }
    extract_i64_array_from_gguf(path, &format!("llama.{}", suffix))
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

/// Архитектура модели из метаданных GGUF (`general.architecture`).
///
/// Современные GGUF хранят гиперпараметры под префиксом архитектуры:
/// `qwen35.block_count`, `granite.attention.head_count`, ... — ключей `llama.*`
/// в таких файлах НЕТ. Префикс нужен для `extract_u32_with_arch`.
pub fn extract_gguf_arch(path: &str) -> Option<String> {
    extract_string_from_gguf(path, "general.architecture")
}

/// Читает u32-ключ GGUF с учётом префикса архитектуры.
///
/// Сначала пробует `<arch>.<suffix>` (современная спецификация GGUF), затем
/// legacy `llama.<suffix>` (старые модели). Ни один не найден → `None`.
pub fn extract_u32_with_arch(path: &str, arch: Option<&str>, suffix: &str) -> Option<u32> {
    if let Some(arch) = arch.filter(|a| !a.is_empty()) {
        if let Some(v) = extract_u32_from_gguf(path, &format!("{}.{}", arch, suffix)) {
            return Some(v);
        }
    }
    extract_u32_from_gguf(path, &format!("llama.{}", suffix))
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

    fn kv_i64_array(key: &str, vals: &[i64]) -> Vec<u8> {
        let mut b = Buf::new();
        b.string(key);
        b.u32(9);          // GGUF_TYPE_ARRAY
        b.u32(12);         // элемент массива: INT64
        b.u64(vals.len() as u64);
        for v in vals { b.v.extend_from_slice(&v.to_le_bytes()); }
        b.v
    }

    fn kv_bool_array(key: &str, vals: &[i64]) -> Vec<u8> {
        let mut b = Buf::new();
        b.string(key);
        b.u32(9);          // GGUF_TYPE_ARRAY
        b.u32(7);          // элемент массива: BOOL
        b.u64(vals.len() as u64);
        for v in vals { b.v.push(if *v != 0 { 1 } else { 0 }); }
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
        build_gguf_with("testarch", &[], declared_blocks, tensors)
    }

    /// GGUF с произвольным `arch` и дополнительными KV-парами (`extra_kv`).
    /// База: `general.architecture`, `<arch>.block_count`, `general.alignment`.
    fn build_gguf_with(arch: &str, extra_kv: &[Vec<u8>], declared_blocks: u32, tensors: &[TensorSpec]) -> Vec<u8> {
        let mut kv = Vec::new();
        kv.extend_from_slice(&kv_string("general.architecture", arch));
        kv.extend_from_slice(&kv_u32(&format!("{}.block_count", arch), declared_blocks));
        kv.extend_from_slice(&kv_u32("general.alignment", 32));
        for e in extra_kv { kv.extend_from_slice(e); }

        let mut header = Buf::new();
        header.v.extend_from_slice(b"GGUF");
        header.u32(3);
        header.u64(tensors.len() as u64);
        header.u64((3 + extra_kv.len()) as u64);
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

    // ── Новые helpers: extract_gguf_arch / extract_u32_with_arch ──

    #[test]
    fn extract_arch_and_keys_with_arch_prefix() {
        let extra = vec![
            kv_u32("qwen35.attention.head_count", 24),
            kv_u32("qwen35.attention.head_count_kv", 4),
            kv_u32("qwen35.embedding_length", 5120),
            kv_u32("qwen35.attention.key_length", 256),
            kv_u32("qwen35.attention.value_length", 256),
            kv_u32("qwen35.full_attention_interval", 4),
        ];
        let bytes = build_gguf_with("qwen35", &extra, 64, &valid_tensors());
        let file = TempFile::new("arch_qwen35", &bytes);
        let p = file.path.to_str().unwrap();

        assert_eq!(extract_gguf_arch(p).as_deref(), Some("qwen35"));
        let arch = extract_gguf_arch(p);
        assert_eq!(extract_u32_with_arch(p, arch.as_deref(), "block_count"), Some(64));
        assert_eq!(extract_u32_with_arch(p, arch.as_deref(), "attention.head_count"), Some(24));
        assert_eq!(extract_u32_with_arch(p, arch.as_deref(), "attention.head_count_kv"), Some(4));
        assert_eq!(extract_u32_with_arch(p, arch.as_deref(), "attention.key_length"), Some(256));
        assert_eq!(extract_u32_with_arch(p, arch.as_deref(), "full_attention_interval"), Some(4));
        // Ключа нет ни под префиксом, ни под llama.* → None (не дефолт).
        assert_eq!(extract_u32_with_arch(p, arch.as_deref(), "attention.scale"), None);
    }

    #[test]
    fn extract_u32_with_arch_falls_back_to_llama_prefix() {
        // У модели arch = gemma3, но гиперпараметры лежат под legacy llama.*.
        let extra = vec![kv_u32("llama.embedding_length", 4096)];
        let bytes = build_gguf_with("gemma3", &extra, 8, &valid_tensors());
        let file = TempFile::new("arch_legacy", &bytes);
        let p = file.path.to_str().unwrap();

        assert_eq!(extract_gguf_arch(p).as_deref(), Some("gemma3"));
        let arch = extract_gguf_arch(p);
        // <arch>.embedding_length отсутствует → берём llama.embedding_length.
        assert_eq!(extract_u32_with_arch(p, arch.as_deref(), "embedding_length"), Some(4096));
    }

    #[test]
    fn extract_u32_with_arch_without_arch_uses_llama_only() {
        let extra = vec![kv_u32("llama.block_count", 12)];
        let bytes = build_gguf_with("gemma3", &extra, 8, &valid_tensors());
        let file = TempFile::new("arch_noarch", &bytes);
        let p = file.path.to_str().unwrap();
        // arch=None → только llama.* (архитектура не прочиталась — не бывает в валидных файлах).
        assert_eq!(extract_u32_with_arch(p, None, "block_count"), Some(12));
        assert_eq!(extract_u32_with_arch(p, None, "attention.head_count"), None);
    }

    // ── Массивы i64 (head_count_kv как ARRAY — gemma4 и др.) ──

    #[test]
    fn extract_i64_array_with_arch_reads_array_key() {
        // gemma4-style: head_count_kv — массив 48 значений (8 для 40 SWA-слоёв,
        // 1 для 8 dense-слоёв). Скалярный читатель head_count_kv вернул бы None —
        // это и был корень завышенного KV (фоллбэк heads=16). Обрати внимание:
        // у gemma4 SWA-слои несут БОЛЬШЕ KV-голов, чем dense (инверсия от того,
        // как обычно устроены гибриды) — граница определяется sliding_window_pattern.
        let mut kv_row: Vec<i64> = vec![8; 40];
        kv_row.extend(vec![1; 8]);
        let extra = vec![kv_i64_array("gemma4.attention.head_count_kv", &kv_row)];
        let bytes = build_gguf_with("gemma4", &extra, 48, &valid_tensors());
        let file = TempFile::new("array_gemma4", &bytes);
        let p = file.path.to_str().unwrap();

        let arch = extract_gguf_arch(p);
        assert_eq!(arch.as_deref(), Some("gemma4"));
        let arr = extract_i64_array_with_arch(p, arch.as_deref(), "attention.head_count_kv").unwrap();
        assert_eq!(arr.len(), 48, "массив должен содержать 48 значений (по слою)");
        assert!(arr[..40].iter().all(|&v| v == 8), "первые 40 (SWA): {arr:?}");
        assert!(arr[40..].iter().all(|&v| v == 1), "последние 8 (dense): {arr:?}");
        // Скалярный extract_u32_with_arch НЕ должен находить массив (None, а не дефолт).
        assert_eq!(extract_u32_with_arch(p, arch.as_deref(), "attention.head_count_kv"), None);
    }

    #[test]
    fn extract_i64_array_with_arch_falls_back_to_llama_prefix() {
        let extra = vec![kv_i64_array("llama.attention.head_count_kv", &[4, 4, 1])];
        let bytes = build_gguf_with("gemma4", &extra, 3, &valid_tensors());
        let file = TempFile::new("array_legacy", &bytes);
        let p = file.path.to_str().unwrap();

        let arch = extract_gguf_arch(p);
        let arr = extract_i64_array_with_arch(p, arch.as_deref(), "attention.head_count_kv").unwrap();
        assert_eq!(arr, vec![4, 4, 1]);
    }

    #[test]
    fn extract_i64_array_scalar_key_returns_none() {
        // head_count_kv хранится скаляром u32 (обычная GQA-модель) — массив не читается.
        let extra = vec![kv_u32("qwen35.attention.head_count_kv", 4)];
        let bytes = build_gguf_with("qwen35", &extra, 8, &valid_tensors());
        let file = TempFile::new("array_scalar", &bytes);
        let p = file.path.to_str().unwrap();

        let arch = extract_gguf_arch(p);
        assert_eq!(extract_i64_array_with_arch(p, arch.as_deref(), "attention.head_count_kv"), None);
        // Зато скаляр читается как раньше.
        assert_eq!(extract_u32_with_arch(p, arch.as_deref(), "attention.head_count_kv"), Some(4));
    }

    // ── Интеграция: estimate_vram читает arch-ключи и KV считает по GQA/hybrid ──

    #[test]
    fn estimate_vram_uses_arch_prefixed_gqa_hybrid_kv() {
        // qwen35-гибрид как в Ternary-Bonsai-27B: 64 блока, attention в каждом 4-м,
        // heads_kv=4, key/value_len=256 → attn_layers=16.
        let extra = vec![
            kv_u32("qwen35.attention.head_count", 24),
            kv_u32("qwen35.attention.head_count_kv", 4),
            kv_u32("qwen35.embedding_length", 5120),
            kv_u32("qwen35.attention.key_length", 256),
            kv_u32("qwen35.attention.value_length", 256),
            kv_u32("qwen35.full_attention_interval", 4),
        ];
        let bytes = build_gguf_with("qwen35", &extra, 64, &valid_tensors());
        let file = TempFile::new("vram_hybrid", &bytes);
        let p = file.path.to_str().unwrap();

        let est = crate::infra::vram_estimate::estimate_vram(p, 24576, false, false);
        // KV = 16 · 24576 · 4 · (256+256)·2 байт = 1 610 612 736 = 1536 МБ.
        assert!((est.kv_mb - 1536.0).abs() < 0.01, "KV: {} МБ (ожидалось 1536)", est.kv_mb);
        // Старые дефолты (32 слоя, heads 32, heads_kv 32, embd 4096) дали бы ≥12 288 МБ.
        assert!(est.kv_mb < 2000.0, "KV всё ещё завышен: {} МБ", est.kv_mb);
    }

    #[test]
    fn estimate_vram_falls_back_to_llama_prefix() {
        let extra = vec![
            kv_u32("llama.block_count", 8),
            kv_u32("llama.attention.head_count", 32),
            kv_u32("llama.attention.head_count_kv", 4),
            kv_u32("llama.embedding_length", 4096),
        ];
        let bytes = build_gguf_with("gemma3", &extra, 8, &valid_tensors());
        let file = TempFile::new("vram_legacy", &bytes);
        let p = file.path.to_str().unwrap();

        // arch=gemma3, но gemma3.* параметров нет → читаются llama.*:
        // layers=8, heads_kv=4, key/value=embd/heads=128, KV=8·1024·4·512=16 МБ.
        let est = crate::infra::vram_estimate::estimate_vram(p, 1024, false, false);
        assert!((est.kv_mb - 16.0).abs() < 0.01, "KV: {} МБ (ожидалось 16)", est.kv_mb);
    }

    #[test]
    fn estimate_vram_per_layer_kv_array_gemma4() {
        // gemma4-12B: 48 слоёв (40 SWA + 8 dense), head_count_kv — массив
        // (8 для SWA, 1 для dense — инверсия от классических гибридов),
        // key/value 512, SWA 256, window 1024. Граница SWA — sliding_window_pattern.
        let mut kv_row: Vec<i64> = vec![8; 40];
        kv_row.extend(vec![1; 8]);
        let mut swa_row: Vec<i64> = vec![1; 40];
        swa_row.extend(vec![0; 8]);
        let extra = vec![
            kv_i64_array("gemma4.attention.head_count_kv", &kv_row),
            kv_bool_array("gemma4.attention.sliding_window_pattern", &swa_row),
            kv_u32("gemma4.attention.sliding_window", 1024),
            kv_u32("gemma4.attention.head_count", 16),
            kv_u32("gemma4.embedding_length", 7168),
            kv_u32("gemma4.attention.key_length", 512),
            kv_u32("gemma4.attention.value_length", 512),
            kv_u32("gemma4.attention.key_length_swa", 256),
            kv_u32("gemma4.attention.value_length_swa", 256),
        ];
        let bytes = build_gguf_with("gemma4", &extra, 48, &valid_tensors());
        let file = TempFile::new("vram_gemma4", &bytes);
        let p = file.path.to_str().unwrap();

        // Dense: 8·1·(512+512)·2 = 16384 Б/токен; SWA: 40·8·(256+256)·2 = 327680 Б/ячейку.
        // ctx 1024 (n_ctx_seq=1024, SWA cells = min(1024, 1024·4+512)=1024):
        //   dense 16384·1024 = 16 MiB + SWA 327680·1024 = 320 MiB → 336 MiB.
        let est = crate::infra::vram_estimate::estimate_vram(p, 1024, false, false);
        assert!((est.kv_mb - 336.0).abs() < 0.01, "KV: {} МБ (ожидалось 336)", est.kv_mb);
        assert_eq!(est.num_attn_layers, 48);

        // ctx 18204 (n_ctx_seq=18432; SWA cells = min(18432, 4608)=4608):
        //   dense 16384·18432 = 288 MiB + SWA 327680·4608 = 1440 MiB → 1728 MiB.
        let est2 = crate::infra::vram_estimate::estimate_vram(p, 18_204, false, false);
        assert!((est2.kv_mb - 1728.0).abs() < 0.01, "KV: {} МБ (ожидалось 1728)", est2.kv_mb);
        assert!((est2.kv_swa_mb - 1440.0).abs() < 0.01, "SWA KV: {} МБ (ожидалось 1440)", est2.kv_swa_mb);
    }

    #[test]
    fn estimate_vram_gemma4_without_pattern_falls_back_to_smaller_head_swa() {
        // Паттерна нет (гибриды вроде gemma3n): признак SWA = меньше KV-голов.
        // Здесь 8-head слои «перенесём» в dense, а 1-head — в SWA, чтобы проверить
        // fallback (порядок слоёв: [8;40] dense + [1;8] SWA — наоборот от gemma4).
        let mut kv_row: Vec<i64> = vec![8; 40];
        kv_row.extend(vec![1; 8]);
        let extra = vec![
            kv_i64_array("gemma4.attention.head_count_kv", &kv_row),
            kv_u32("gemma4.attention.sliding_window", 1024),
            kv_u32("gemma4.attention.head_count", 16),
            kv_u32("gemma4.embedding_length", 7168),
            kv_u32("gemma4.attention.key_length", 512),
            kv_u32("gemma4.attention.value_length", 512),
            kv_u32("gemma4.attention.key_length_swa", 256),
            kv_u32("gemma4.attention.value_length_swa", 256),
        ];
        let bytes = build_gguf_with("gemma4", &extra, 48, &valid_tensors());
        let file = TempFile::new("vram_gemma4_nopattern", &bytes);
        let p = file.path.to_str().unwrap();

        // Fallback: 1-head слои считаются SWA (8 шт), 8-head — dense (40 шт):
        //   dense 40·8·(512+512)·2 = 655360 Б/токен; SWA 8·1·(256+256)·2 = 8192 Б/ячейку.
        // ctx 1024: dense 655360·1024 = 640 MiB + SWA 8192·1024 = 8 MiB → 648 MiB.
        let est = crate::infra::vram_estimate::estimate_vram(p, 1024, false, false);
        assert!((est.kv_mb - 648.0).abs() < 0.01, "KV: {} МБ (ожидалось 648)", est.kv_mb);
    }

    #[test]
    fn estimate_vram_total_includes_buffers_minus_kv() {
        // total = модель + KV + буферы (≤256) — буферы отделены от KV.
        let extra = vec![
            kv_u32("qwen35.attention.head_count", 24),
            kv_u32("qwen35.attention.head_count_kv", 4),
            kv_u32("qwen35.embedding_length", 5120),
            kv_u32("qwen35.attention.key_length", 256),
            kv_u32("qwen35.attention.value_length", 256),
            kv_u32("qwen35.full_attention_interval", 4),
        ];
        let bytes = build_gguf_with("qwen35", &extra, 64, &valid_tensors());
        let file = TempFile::new("vram_total", &bytes);
        let p = file.path.to_str().unwrap();

        let est = crate::infra::vram_estimate::estimate_vram(p, 24576, false, false);
        let file_mb = bytes.len() as f64 / (1024.0 * 1024.0);
        assert!((est.model_mb - file_mb).abs() < 0.01, "model_mb: {}", est.model_mb);
        assert!((est.buffers_mb - ((est.model_mb + est.kv_mb) * 0.10).min(256.0)).abs() < 0.01);
        assert!((est.total_mb - (est.model_mb + est.kv_mb + est.buffers_mb)).abs() < 0.01, "total: {}", est.total_mb);
    }
}
