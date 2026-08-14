use image::{Rgba, RgbaImage};
use std::path::PathBuf;
use std::sync::OnceLock;

const CIRCLE_REF: [i32; 3] = [86, 110, 146];
const RING_REF:   [i32; 3] = [242, 244, 247];

fn dir() -> PathBuf { crate::app_dir().join("icons") }

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

fn hex_to_rgb(hex: &str) -> Option<[u8; 3]> {
    let h = hex.trim_start_matches('#');
    if h.len() != 6 { return None; }
    let r = u8::from_str_radix(&h[0..2], 16).ok()?;
    let g = u8::from_str_radix(&h[2..4], 16).ok()?;
    let b = u8::from_str_radix(&h[4..6], 16).ok()?;
    Some([r, g, b])
}

fn rgb_to_hcl(c: [u8; 3]) -> (f32, f32, f32) {
    let [r, g, b] = c.map(|v| v as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    if d == 0.0 { return (0.0, 0.0, (max + min) / 2.0); }
    let h = 60.0 * if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h, d, (max + min) / 2.0)
}

fn hcl_to_rgb(h: f32, c: f32, l: f32) -> [u8; 3] {
    let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match h as u32 / 60 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [r, g, b].map(|v| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8)
}

fn palette_rgb(c: [u8; 3]) -> [u8; 3] {
    const L_MIN: f32 = 0.25;
    const L_MAX: f32 = 0.75;
    const CHROMA_MAX: f32 = 0.85;

    let (h, t, l_in)  = rgb_to_hcl(c);
    let (_, _, l_ref) = rgb_to_hcl([CIRCLE_REF[0] as u8, CIRCLE_REF[1] as u8, CIRCLE_REF[2] as u8]);

    let chroma = CHROMA_MAX * t;
    let l = (l_ref * t + l_in * (1.0 - t)).clamp(L_MIN, L_MAX);
    hcl_to_rgb(h, chroma, l)
}

fn generate(hex: &str) -> RgbaImage {
    let (base_img, mask) = base();
    let mut out = base_img.clone();
    if let Some(rgb) = hex_to_rgb(hex) {
        let target = palette_rgb(rgb);
        for (px, &circle) in out.pixels_mut().zip(mask) {
            if circle {
                let a = px.0[3];
                *px = Rgba([target[0], target[1], target[2], a]);
            }
        }
    }
    out
}

pub fn icon_path_for(hex: &str) -> PathBuf {
    let hex  = hex.trim_start_matches('#').to_ascii_lowercase();
    let path = dir().join(format!("{hex}.png"));
    if !path.exists() {
        let _ = std::fs::create_dir_all(dir());
        let _ = generate(&hex).save(&path);
    }
    path
}
