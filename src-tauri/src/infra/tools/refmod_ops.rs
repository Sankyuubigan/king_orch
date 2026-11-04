//! Операции редактирования кадров RefMod-карточек (tasks/261104_0230-db2cb59c,
//! Этап 2 Шаг 1): телосложение (warp), цвет (hue), сборка из двух карточек
//! (splice). Чистые трансформации над кадрами в памяти; нормализация обратно
//! на сетку 32px — через SSOT-`normalize_frame` плагина (та же математика, что
//! и при первом сохранении карточки). Результат edit — НОВАЯ карточка
//! (`<slug>_v2`, `<slug>_v3`, …) либо перезапись по явному `overwrite: true`:
//! исходник не трогаем (реверсибельность).

use serde_json::Value;
use std::path::Path;

use crate::infra::tools::ToolError;
use crate::infra::tools::refmod::RefModBundle;

/// Вилка фактора телосложения (горизонтальный масштаб кадров).
pub const WARP_FACTOR_MIN: f32 = 0.7;
pub const WARP_FACTOR_MAX: f32 = 1.3;
/// Предел сдвига оттенка (цвет волос/кожи).
pub const HUE_DEGREES_MAX: i32 = 40;
/// Ширина растушёвки шва splice (одна ячейка сетки латентов).
pub const SPLICE_FEATHER_PX: u32 = 32;
/// Максимум операций в одном refmod_edit (санитарный потолок).
pub const OPS_MAX: usize = 8;

/// Одна операция редактирования (разобранный JSON `ops`).
#[derive(Clone, Debug, PartialEq)]
pub enum RefModOp {
    /// Горизонтальный масштаб кадров (телосложение): factor > 1 — шире.
    Warp { factor: f32 },
    /// Сдвиг оттенка (цвет волос/кожи/одежды), градусы −40..40.
    Hue { degrees: i32 },
    /// Вертикальная склейка с кадрами другой карточки попарно, с растушёвкой шва.
    Splice { other_slug: String, position: SplicePosition },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplicePosition {
    /// Кадры другой карточки СВЕРХУ.
    Top,
    /// Кадры другой карточки СНИЗУ.
    Bottom,
}

/// Разобрать и провалидировать массив `ops` (гейты ДО любых обращений к диску).
pub fn parse_ops(value: Option<&Value>) -> Result<Vec<RefModOp>, ToolError> {
    let Some(list) = value else {
        return Err(ToolError::Usage(
            "refmod_edit: обязателен массив ops — хотя бы одна операция \
             {\"op\": \"warp\"|\"hue\"|\"splice\", ...}."
                .to_string(),
        ));
    };
    let Some(list) = list.as_array() else {
        return Err(ToolError::Usage(
            "refmod_edit: ops должен быть массивом операций [{\"op\": ...}, ...]".to_string(),
        ));
    };
    if list.is_empty() {
        return Err(ToolError::Usage(
            "refmod_edit: ops пуст — нужна хотя бы одна операция".to_string(),
        ));
    }
    if list.len() > OPS_MAX {
        return Err(ToolError::Usage(format!(
            "refmod_edit: максимум {} операций за вызов, передано {}.",
            OPS_MAX,
            list.len()
        )));
    }
    list.iter()
        .map(|item| {
            let op = item
                .get("op")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::Usage(format!(
                    "refmod_edit: у операции нет поля op (\"warp\"|\"hue\"|\"splice\"): {}",
                    item
                )))?;
            match op {
                "warp" => {
                    let factor = item.get("factor").and_then(|v| v.as_f64()).ok_or_else(|| {
                        ToolError::Usage("refmod_edit: warp требует factor (число)".to_string())
                    })? as f32;
                    if !(WARP_FACTOR_MIN..=WARP_FACTOR_MAX).contains(&factor) {
                        return Err(ToolError::Usage(format!(
                            "refmod_edit: factor {} вне вилки {}–{}.",
                            factor, WARP_FACTOR_MIN, WARP_FACTOR_MAX
                        )));
                    }
                    Ok(RefModOp::Warp { factor })
                }
                "hue" => {
                    let degrees = item.get("degrees").and_then(|v| v.as_i64()).ok_or_else(|| {
                        ToolError::Usage("refmod_edit: hue требует degrees (целое)".to_string())
                    })? as i32;
                    if degrees.abs() > HUE_DEGREES_MAX {
                        return Err(ToolError::Usage(format!(
                            "refmod_edit: degrees {} вне вилки ±{}.",
                            degrees, HUE_DEGREES_MAX
                        )));
                    }
                    Ok(RefModOp::Hue { degrees })
                }
                "splice" => {
                    let other_slug = item
                        .get("other_slug")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            ToolError::Usage(
                                "refmod_edit: splice требует other_slug (slug другой карточки)"
                                    .to_string(),
                            )
                        })?
                        .to_string();
                    let position = match item.get("position").and_then(|v| v.as_str()) {
                        Some("top") => SplicePosition::Top,
                        Some("bottom") => SplicePosition::Bottom,
                        _ => {
                            return Err(ToolError::Usage(
                                "refmod_edit: splice требует position \"top\" или \"bottom\" \
                                 (где окажутся кадры другой карточки)."
                                    .to_string(),
                            ))
                        }
                    };
                    Ok(RefModOp::Splice {
                        other_slug,
                        position,
                    })
                }
                other => Err(ToolError::Usage(format!(
                    "refmod_edit: неизвестная операция '{}'. Доступны: warp, hue, splice.",
                    other
                ))),
            }
        })
        .collect()
}

/// Загрузить кадры другой карточки в память (для splice).
pub fn load_other_frames(other_slug: &str) -> Result<Vec<image::DynamicImage>, ToolError> {
    load_other_frames_in(&crate::infra::tools::refmod::refmods_dir(), other_slug)
}

/// Герметичный вариант: кадры другой карточки из указанного корня.
pub fn load_other_frames_in(
    refmods_root: &Path,
    other_slug: &str,
) -> Result<Vec<image::DynamicImage>, ToolError> {
    let other: RefModBundle =
        crate::infra::tools::refmod::load_refmod_card_in(refmods_root, other_slug)?;
    other
        .frames
        .iter()
        .map(|path| {
            image::open(path).map_err(|e| {
                ToolError::Io(format!("refmod_edit: не удалось прочитать кадр {}: {}", path.display(), e))
            })
        })
        .collect()
}

/// Применить операции к набору кадров последовательно. `other_frames` — загрузчик
/// кадров другой карточки для splice (инъекция вместо прямого чтения — герметично
/// для тестов). Splice заменяет набор попарной склейкой (min из двух длин).
pub fn apply_ops(
    frames: Vec<image::DynamicImage>,
    ops: &[RefModOp],
    other_frames: &dyn Fn(&str) -> Result<Vec<image::DynamicImage>, ToolError>,
) -> Result<Vec<image::DynamicImage>, ToolError> {
    let mut current = frames;
    for op in ops {
        match op {
            RefModOp::Warp { factor } => {
                current = current
                    .into_iter()
                    .map(|img| apply_warp(img, *factor))
                    .collect();
            }
            RefModOp::Hue { degrees } => {
                current = current
                    .into_iter()
                    .map(|img| apply_hue(img, *degrees))
                    .collect();
            }
            RefModOp::Splice {
                other_slug,
                position,
            } => {
                let other = other_frames(other_slug)?;
                current = splice_sets(&current, &other, *position)?;
            }
        }
    }
    Ok(current)
}

/// Телосложение: горизонтальный масштаб кадра с возвратом на сетку 32px/1024px.
pub fn apply_warp(img: image::DynamicImage, factor: f32) -> image::DynamicImage {
    let (w, h) = (img.width(), img.height());
    let new_w = ((w as f32) * factor).round().max(1.0) as u32;
    let scaled = img.resize_exact(
        new_w,
        h,
        image::imageops::FilterType::Lanczos3,
    );
    super::refmod::normalize_frame(scaled)
}

/// Цвет: сдвиг оттенка (huerotate сохраняет габариты — сетка не ломается).
pub fn apply_hue(img: image::DynamicImage, degrees: i32) -> image::DynamicImage {
    img.huerotate(degrees)
}

/// Вертикальная склейка двух наборов попарно: кадр_i результата = кадр_i верха +
/// кадр_i низа с растушёванным швом. Кадров в результате — min из двух наборов
/// (структурно честно: пары без пары не выдумываем).
pub fn splice_sets(
    top: &[image::DynamicImage],
    bottom: &[image::DynamicImage],
    position: SplicePosition,
) -> Result<Vec<image::DynamicImage>, ToolError> {
    let (first, second) = match position {
        SplicePosition::Top => (bottom, top),    // other сверху
        SplicePosition::Bottom => (top, bottom), // other снизу
    };
    if first.is_empty() || second.is_empty() {
        return Err(ToolError::Usage(
            "refmod_edit: splice невозможен — один из наборов кадров пуст".to_string(),
        ));
    }
    Ok(first
        .iter()
        .zip(second.iter())
        .map(|(a, b)| splice_vertical(a, b))
        .collect())
}

/// Вертикальная склейка двух кадров с альфа-градиентом по шву:
/// выравниваем по ширине (max из двух), склеиваем, шов B=32px — линейный
/// переход; итог нормируется SSOT-`normalize_frame` (≤1024px, сетка 32).
fn splice_vertical(a: &image::DynamicImage, b: &image::DynamicImage) -> image::DynamicImage {
    let width = a.width().max(b.width());
    let fit = |img: &image::DynamicImage| -> image::DynamicImage {
        if img.width() == width {
            img.clone()
        } else {
            let new_h = ((img.height() as f32) * (width as f32 / img.width() as f32)).round().max(1.0) as u32;
            img.resize_exact(width, new_h, image::imageops::FilterType::Lanczos3)
                .to_rgb8()
                .into()
        }
    };
    let top = fit(a).to_rgba8();
    let bottom = fit(b).to_rgba8();
    let feather = SPLICE_FEATHER_PX.min(top.height().saturating_sub(1)).min(bottom.height().saturating_sub(1));
    let (w, th, bh) = (top.width(), top.height(), bottom.height());
    let out_h = th + bh - feather;
    let mut out = image::RgbaImage::new(w, out_h);
    // Верх целиком, кроме полосы шва.
    for y in 0..th - feather {
        for x in 0..w {
            out.put_pixel(x, y, *top.get_pixel(x, y));
        }
    }
    // Полоса шва: линейный переход верх → низ.
    for i in 0..feather {
        let y = th - feather + i;
        let t = (i + 1) as f32 / (feather + 1) as f32; // 0..1 сверху вниз
        for x in 0..w {
            let p_top = top.get_pixel(x, y);
            let p_bot = bottom.get_pixel(x, i);
            let blend = |u: u8, v: u8| -> u8 {
                (u as f32 * (1.0 - t) + v as f32 * t).round() as u8
            };
            out.put_pixel(
                x,
                y,
                image::Rgba([
                    blend(p_top[0], p_bot[0]),
                    blend(p_top[1], p_bot[1]),
                    blend(p_top[2], p_bot[2]),
                    blend(p_top[3], p_bot[3]),
                ]),
            );
        }
    }
    // Низ целиком, кроме полосы шва.
    for y in feather..bh {
        for x in 0..w {
            out.put_pixel(x, th + y - feather, *bottom.get_pixel(x, y));
        }
    }
    super::refmod::normalize_frame(image::DynamicImage::ImageRgba8(out))
}

/// Следующий свободный slug-вариант: `<base>`, `<base>_v2`, `<base>_v3`, …
/// Если вариант перестаёт влезать в валидатор slug — честный Usage.
pub fn next_free_slug(base: &str) -> Result<String, ToolError> {
    next_free_slug_in(&crate::infra::tools::refmod::refmods_dir(), base)
}

/// Герметичный вариант: поиск свободного slug в указанном корне.
pub fn next_free_slug_in(refmods_root: &Path, base: &str) -> Result<String, ToolError> {
    if !refmods_root.join(base).exists() {
        return Ok(base.to_string());
    }
    for v in 2..100 {
        let candidate = format!("{}_v{}", base, v);
        if !refmods_root.join(&candidate).exists() {
            return Ok(candidate);
        }
    }
    Err(ToolError::Usage(format!(
        "refmod_edit: у карточки '{}' слишком много вариантов (v2..v99 заняты) — \
         укажи целевой slug явно через overwrite.",
        base
    )))
}

#[cfg(test)]
#[path = "refmod_ops_tests.rs"]
mod refmod_ops_tests;
