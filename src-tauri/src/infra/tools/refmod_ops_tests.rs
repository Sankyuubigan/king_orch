//! Тесты операций редактирования RefMod-карточек: гейты parse_ops до диска,
//! warp/hue/splice держат сетку 32px, splice — попарная склейка min-набора.

use super::*;
use serde_json::json;
use crate::infra::tools::refmod::{CARD_PX_BUDGET_MAIN, FRAME_SIZE_MULTIPLE};

fn solid(w: u32, h: u32, color: [u8; 3]) -> image::DynamicImage {
    image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
        w,
        h,
        image::Rgb(color),
    ))
}

// ── parse_ops: гейты ДО любых обращений к диску ──

#[test]
fn parse_ops_valid_mix() {
    let ops = parse_ops(Some(&serde_json::json!([
        {"op": "warp", "factor": 0.85},
        {"op": "hue", "degrees": -20},
        {"op": "splice", "other_slug": "x", "position": "bottom"}
    ])))
    .expect("валидный набор разбирается");
    assert_eq!(
        ops,
        vec![
            RefModOp::Warp { factor: 0.85 },
            RefModOp::Hue { degrees: -20 },
            RefModOp::Splice { other_slug: "x".to_string(), position: SplicePosition::Bottom },
        ]
    );
}

#[test]
fn parse_ops_rejects_structural_breaks() {
    for (bad, why) in [
        (json!(null), "ops отсутствует"),
        (json!({}), "ops не массив"),
        (json!([]), "ops пуст"),
    ] {
        let err = parse_ops(Some(&bad)).unwrap_err();
        assert!(matches!(err, ToolError::Usage(_)), "{} → {:?}", why, err);
    }
}

#[test]
fn parse_ops_rejects_overflow() {
    let many: Vec<Value> = (0..9)
        .map(|i| serde_json::json!({"op": "hue", "degrees": i}))
        .collect();
    let err = parse_ops(Some(&Value::Array(many))).unwrap_err();
    assert!(
        matches!(err, ToolError::Usage(ref m) if m.contains("максимум")),
        "{:?}",
        err
    );
}

#[test]
fn parse_ops_validates_fields() {
    for (bad, why) in [
        (json!([{}]), "нет поля op"),
        (json!([{"op": "rotate"}]), "неизвестная op"),
        (json!([{"op": "warp"}]), "warp без factor"),
        (json!([{"op": "warp", "factor": 0.5}]), "warp ниже вилки"),
        (json!([{"op": "warp", "factor": 1.5}]), "warp выше вилки"),
        (json!([{"op": "hue"}]), "hue без degrees"),
        (json!([{"op": "hue", "degrees": 50}]), "hue выше вилки"),
        (json!([{"op": "splice", "position": "top"}]), "splice без other_slug"),
        (json!([{"op": "splice", "other_slug": "x"}]), "splice без position"),
    ] {
        let err = parse_ops(Some(&bad)).unwrap_err();
        assert!(matches!(err, ToolError::Usage(_)), "{} → {:?}", why, err);
    }
}

// ── Поведение операций: сетка 32px и честная склейка ──

#[test]
fn warp_keeps_grid() {
    for factor in [0.7, 0.85, 1.0, 1.15, 1.3] {
        let out = apply_warp(solid(128, 96, [200, 150, 100]), factor);
        let (w, h) = (out.width(), out.height());
        assert_eq!(w % FRAME_SIZE_MULTIPLE, 0, "factor={} ширина {}", factor, w);
        assert_eq!(h % FRAME_SIZE_MULTIPLE, 0, "factor={} высота {}", factor, h);
        assert!(
            w as u64 * h as u64 <= CARD_PX_BUDGET_MAIN,
            "factor={} кап площади главного нарушен",
            factor
        );
    }
    // Крупный кадр при сжатии возвращается в потолок SSOT (бюджет площади).
    let out = apply_warp(solid(2048, 1024, [100, 100, 100]), 0.7);
    let (w, h) = (out.width(), out.height());
    assert_eq!(w % FRAME_SIZE_MULTIPLE, 0);
    assert_eq!(h % FRAME_SIZE_MULTIPLE, 0);
    assert!(w as u64 * h as u64 <= CARD_PX_BUDGET_MAIN);
}

#[test]
fn hue_keeps_dims_and_grid() {
    let out = apply_hue(solid(128, 96, [200, 150, 100]), -20);
    assert_eq!((out.width(), out.height()), (128, 96), "huerotate сохраняет габариты");
    let out = apply_hue(solid(1024, 512, [120, 180, 200]), 40);
    assert_eq!((out.width(), out.height()), (1024, 512));
}

#[test]
fn splice_pairwise_min_count() {
    let ours = vec![solid(32, 64, [255, 0, 0]), solid(32, 64, [0, 255, 0])];
    let other = vec![solid(32, 64, [0, 0, 255])];
    let merged = splice_sets(&ours, &other, SplicePosition::Bottom)
        .expect("склейка непустых наборов");
    assert_eq!(merged.len(), 1, "кадров = min(2,1) — пары без пары не выдумываются");
}

#[test]
fn splice_other_goes_where_asked() {
    let current = vec![solid(32, 64, [255, 0, 0])];
    let other = vec![solid(32, 64, [0, 0, 255])];
    // position Bottom → other снизу: верх чисто красный, низ чисто синий.
    let merged = splice_sets(&current, &other, SplicePosition::Bottom).unwrap();
    assert_eq!((merged[0].width(), merged[0].height()), (32, 96), "64+64 минус шов");
    let px = merged[0].to_rgb8();
    assert_eq!(px.get_pixel(0, 5), &image::Rgb([255, 0, 0]), "верх = текущие");
    assert_eq!(px.get_pixel(0, 90), &image::Rgb([0, 0, 255]), "низ = other");
    // position Top → other сверху: зеркально.
    let merged = splice_sets(&current, &other, SplicePosition::Top).unwrap();
    let px = merged[0].to_rgb8();
    assert_eq!(px.get_pixel(0, 5), &image::Rgb([0, 0, 255]), "верх = other");
    assert_eq!(px.get_pixel(0, 90), &image::Rgb([255, 0, 0]), "низ = текущие");
}

#[test]
fn splice_empty_set_is_honest_error() {
    let full = vec![solid(32, 64, [255, 0, 0])];
    let empty: Vec<image::DynamicImage> = vec![];
    let err = splice_sets(&full, &empty, SplicePosition::Top).unwrap_err();
    assert!(matches!(err, ToolError::Usage(ref m) if m.contains("пуст")), "{:?}", err);
}

// ── next_free_slug_in: варианты v2..v99, переполнение — Usage ──

fn slug_root(tag: &str) -> std::path::PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("king_orch_ops_{}_{}", tag, stamp));
    std::fs::create_dir_all(&root).unwrap();
    root
}

#[test]
fn next_free_slug_walks_variants() {
    let root = slug_root("slug");
    // Свободно → base; base занят → v2; v2 занят → v3.
    assert_eq!(next_free_slug_in(&root, "char").unwrap(), "char");
    std::fs::create_dir_all(root.join("char")).unwrap();
    assert_eq!(next_free_slug_in(&root, "char").unwrap(), "char_v2");
    std::fs::create_dir_all(root.join("char_v2")).unwrap();
    assert_eq!(next_free_slug_in(&root, "char").unwrap(), "char_v3");
}

#[test]
fn next_free_slug_overflow_is_usage() {
    let root = slug_root("slug_full");
    // v2..v99 заняты → честный Usage, а не бесшумная перезапись.
    for v in 2..100 {
        std::fs::create_dir_all(root.join(format!("{}_v{}", "old", v))).unwrap();
    }
    std::fs::create_dir_all(root.join("old")).unwrap();
    let err = next_free_slug_in(&root, "old").unwrap_err();
    assert!(matches!(err, ToolError::Usage(ref m) if m.contains("overwrite")), "{:?}", err);
}
