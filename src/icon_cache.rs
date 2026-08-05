// Кэш перекрашенных иконок под цвет проекта (для тостов уведомлений).
//
// База — единственный ICON_PNG (синий кружок внутри белой буквы C).
// Красим только кружок под hex проекта; букву C не трогаем ни на бит — у
// неё свой едва заметный синеватый оттенок, одинаковый для любого цвета
// проекта (см. обсуждение 2026-08-05). Кружок и буква различаются не по
// оттенку (у чёрных/белых/серых цветов проекта hue не определён), а по
// близости каждого пикселя к одному из двух эталонных цветов — это же
// заодно снимает вопрос про антиалиасинг на границе: смешанные пиксели
// просто уходят к тому эталону, что ближе.
//
// Файлы — app_dir()/icons/<hex>.png, генерируются лениво по требованию.
// meta.json рядом хранит last_used по каждому hex, но НЕ на каждое
// использование (это спам записи на диск) — в памяти всю сессию, на
// диск сбрасывается один раз, при выходе (persist()). Чистка (по
// возрасту и по количеству) — один раз при старте (load()): удалять
// сразу после показа тоста рискованно, Windows может подтянуть файл в
// Action Center и позже.

use image::{Rgba, RgbaImage};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::UNIX_EPOCH;

const MAX_AGE_SECS: u64 = 7 * 24 * 3600;
const MAX_COUNT:    usize = 100;

const CIRCLE_REF: [i32; 3] = [86, 110, 146]; // перекрашиваем
const RING_REF:   [i32; 3] = [242, 244, 247]; // не трогаем

fn dir()       -> PathBuf { crate::app_dir().join("icons") }
fn meta_path() -> PathBuf { dir().join("meta.json") }

fn last_used() -> &'static Mutex<HashMap<String, u64>> {
    static M: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();
    M.get_or_init(|| Mutex::new(HashMap::new()))
}

fn now() -> u64 {
    UNIX_EPOCH.elapsed().map(|d| d.as_secs()).unwrap_or(0)
}

fn normalize(hex: &str) -> String {
    hex.trim_start_matches('#').to_ascii_lowercase()
}

/// Базовая картинка + маска "это кружок?", посчитанная один раз за процесс.
fn base() -> &'static (RgbaImage, Vec<bool>) {
    static B: OnceLock<(RgbaImage, Vec<bool>)> = OnceLock::new();
    B.get_or_init(|| {
        let img = image::load_from_memory(crate::ICON_PNG)
            .expect("встроенная icon.png всегда валидна")
            .to_rgba8();
        let dist = |p: [u8; 3], r: [i32; 3]| {
            (0..3).map(|i| (p[i] as i32 - r[i]).pow(2)).sum::<i32>()
        };
        let mask = img.pixels()
            .map(|p| {
                let [r, g, b, a] = p.0;
                a != 0 && dist([r, g, b], CIRCLE_REF) < dist([r, g, b], RING_REF)
            })
            .collect();
        (img, mask)
    })
}

fn generate(hex: &str) -> RgbaImage {
    let (base_img, mask) = base();
    let mut out = base_img.clone();
    if let Some(c) = crate::project::hex_to_color32(hex) {
        for (px, &circle) in out.pixels_mut().zip(mask) {
            if circle {
                let a = px.0[3];
                *px = Rgba([c.r(), c.g(), c.b(), a]);
            }
        }
    }
    out
}

/// Путь к иконке под цвет проекта. Генерирует и кэширует на диске при
/// первом обращении к данному hex за всё время жизни app_dir.
pub fn icon_path_for(hex: &str) -> PathBuf {
    let hex  = normalize(hex);
    let path = dir().join(format!("{hex}.png"));
    if !path.exists() {
        let _ = std::fs::create_dir_all(dir());
        let _ = generate(&hex).save(&path);
    }
    last_used().lock().unwrap().insert(hex, now());
    path
}

/// Один раз на старте: подтягивает last_used с диска, чистит устаревшее и
/// лишнее. Файлы без записи в meta.json (например, после краша между
/// generate() и persist()) не считаются "древними" — им подставляется
/// mtime самого файла.
pub fn load() {
    let saved: HashMap<String, u64> = std::fs::read_to_string(meta_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    let mut entries: Vec<(String, PathBuf, u64)> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir()) {
        for e in rd.flatten() {
            let path = e.path();
            if path.extension().and_then(|e| e.to_str()) != Some("png") { continue }
            let Some(hex) = path.file_stem().and_then(|s| s.to_str()) else { continue };
            let ts = saved.get(hex).copied().unwrap_or_else(|| {
                e.metadata().and_then(|m| m.modified()).ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0)
            });
            entries.push((hex.to_string(), path, ts));
        }
    }

    let before = entries.len();
    let n = now();
    entries.retain(|(_, path, ts)| {
        n.saturating_sub(*ts) <= MAX_AGE_SECS || { let _ = std::fs::remove_file(path); false }
    });
    if entries.len() > MAX_COUNT {
        entries.sort_by_key(|(_, _, ts)| *ts);
        for (_, path, _) in entries.drain(..entries.len() - MAX_COUNT) {
            let _ = std::fs::remove_file(path);
        }
    }

    let pruned = entries.len() != before;
    *last_used().lock().unwrap() =
        entries.into_iter().map(|(hex, _, ts)| (hex, ts)).collect();
    if pruned { persist(); }
}

/// Один раз на выходе: сбрасывает накопленный за сессию last_used на диск.
pub fn persist() {
    let lu = last_used().lock().unwrap();
    if let Ok(json) = serde_json::to_string(&*lu) {
        let _ = std::fs::create_dir_all(dir());
        let _ = std::fs::write(meta_path(), json);
    }
}
