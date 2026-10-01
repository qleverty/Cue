//! Порядок отображения списка проектов (свитчер и выпадающий список
//! «Всегда открывать»). Физический порядок `Vec<LoadedProject>` не
//! меняется — на нём держатся индексы (active_project_idx и т.д.); здесь
//! только считается, в каком порядке их показывать.
//!
//! Все варианты детерминированны: при равенстве главного ключа сравниваем
//! название без учёта регистра, затем id.

use std::cmp::Ordering;
use eframe::egui::Color32;
use crate::project::LoadedProject;
use crate::settings::ProjectSort;

/// Ниже этой насыщенности цвет считаем нейтральным (серый/чёрный/белый):
/// у него нет осмысленного оттенка (hue), поэтому в радугу он не входит.
const NEUTRAL_SATURATION: f32 = 0.10;

/// HSV в привычных единицах: hue 0..360, saturation и value 0..1.
/// Считается прямо по sRGB-байтам — «как выглядит цвет», без линеаризации.
pub fn hsv(c: Color32) -> (f32, f32, f32) {
    let r = c.r() as f32 / 255.0;
    let g = c.g() as f32 / 255.0;
    let b = c.b() as f32 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d   = max - min;
    let v   = max;
    let s   = if max <= 0.0 { 0.0 } else { d / max };
    let h   = if d <= 0.0 {
        0.0
    } else if max == r {
        60.0 * (((g - b) / d).rem_euclid(6.0))
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    (h, s, v)
}

fn cmp_color(a: Color32, b: Color32) -> Ordering {
    let (ha, sa, va) = hsv(a);
    let (hb, sb, vb) = hsv(b);
    let na = sa < NEUTRAL_SATURATION;
    let nb = sb < NEUTRAL_SATURATION;
    match (na, nb) {
        (false, true)  => Ordering::Less,    // цветные — раньше нейтральных
        (true,  false) => Ordering::Greater,
        // Оба цветные: по hue (радуга), при равенстве — более насыщенные
        // раньше, затем более тёмные раньше.
        (false, false) => ha.total_cmp(&hb)
            .then(sb.total_cmp(&sa))
            .then(va.total_cmp(&vb)),
        // Оба нейтральные: у hue тут нет смысла — от тёмного к светлому.
        (true, true)   => va.total_cmp(&vb),
    }
}

fn cmp_name(a: &LoadedProject, b: &LoadedProject) -> Ordering {
    a.name.to_lowercase().cmp(&b.name.to_lowercase())
        .then_with(|| a.id.cmp(&b.id))
}

/// Индексы `projects` в порядке отображения для выбранной сортировки.
pub fn display_order(projects: &[LoadedProject], mode: ProjectSort) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..projects.len()).collect();
    idx.sort_by(|&i, &j| {
        let (a, b) = (&projects[i], &projects[j]);
        let primary = match mode {
            ProjectSort::ByName     => Ordering::Equal,
            ProjectSort::ByColor    => cmp_color(a.color, b.color),
            ProjectSort::ByCreated  => b.created_at.cmp(&a.created_at),   // новые сверху
            ProjectSort::ByModified => b.last_edited.cmp(&a.last_edited), // недавние сверху
        };
        primary.then_with(|| cmp_name(a, b))
    });
    idx
}

#[cfg(test)]
mod tests {
    use super::*;

    // Палитра проектов из main.rs (PROJECT_PALETTE) — порядок «как в UI».
    const PALETTE: &[(&str, (u8, u8, u8))] = &[
        ("red",    (220, 50, 50)),
        ("orange", (249, 115, 22)),
        ("yellow", (234, 179, 8)),
        ("green",  (34, 197, 94)),
        ("cyan",   (20, 184, 166)),
        ("blue",   (59, 130, 246)),
        ("violet", (168, 85, 247)),
        ("pink",   (236, 72, 153)),
        ("brown",  (139, 90, 43)),
        ("black",  (40, 40, 40)),
        ("white",  (210, 210, 210)),
    ];

    fn proj(id: &str, name: &str, c: Color32, created: u64, edited: u64) -> LoadedProject {
        let mut p = LoadedProject::new(id.into(), name.into(), c, created);
        p.last_edited = edited;
        p
    }

    fn names(ps: &[LoadedProject], order: &[usize]) -> Vec<String> {
        order.iter().map(|&i| ps[i].name.clone()).collect()
    }

    #[test]
    fn hsv_basic_hues() {
        let (h, s, v) = hsv(Color32::from_rgb(255, 0, 0));
        assert_eq!((h, s, v), (0.0, 1.0, 1.0));
        assert!((hsv(Color32::from_rgb(0, 255, 0)).0 - 120.0).abs() < 1e-3);
        assert!((hsv(Color32::from_rgb(0, 0, 255)).0 - 240.0).abs() < 1e-3);
        let (_, s, _) = hsv(Color32::from_rgb(40, 40, 40));
        assert_eq!(s, 0.0);
    }

    #[test]
    fn color_order_over_palette_rainbow_then_neutrals() {
        // Подаём палитру в перемешанном порядке — результат должен быть
        // «радуга, затем нейтральные от тёмного к светлому».
        let mut ps: Vec<LoadedProject> = Vec::new();
        for (i, k) in [9usize, 3, 10, 0, 7, 5, 1, 8, 2, 6, 4].iter().enumerate() {
            let (n, (r, g, b)) = PALETTE[*k];
            ps.push(proj(&format!("id{i}"), n, Color32::from_rgb(r, g, b), 0, 0));
        }
        let order = display_order(&ps, ProjectSort::ByColor);
        assert_eq!(
            names(&ps, &order),
            ["red", "orange", "brown", "yellow", "green", "cyan",
             "blue", "violet", "pink", "black", "white"]
        );
    }

    #[test]
    fn color_ties_resolved_by_name_case_insensitive() {
        let red = Color32::from_rgb(239, 68, 68);
        let ps = vec![
            proj("1", "банан",   red, 0, 0),
            proj("2", "Арбуз",   red, 0, 0),
            proj("3", "Виноград", red, 0, 0),
        ];
        let order = display_order(&ps, ProjectSort::ByColor);
        assert_eq!(names(&ps, &order), ["Арбуз", "банан", "Виноград"]);
    }

    #[test]
    fn near_gray_with_tiny_saturation_counts_as_neutral() {
        // 128,128,130 → saturation ≈ 0.015 < порога: не должен встать
        // среди «красных» из-за произвольного hue.
        let ps = vec![
            proj("1", "gray", Color32::from_rgb(128, 128, 130), 0, 0),
            proj("2", "pink", Color32::from_rgb(236, 72, 153), 0, 0),
        ];
        let order = display_order(&ps, ProjectSort::ByColor);
        assert_eq!(names(&ps, &order), ["pink", "gray"]);
    }

    #[test]
    fn name_sort_is_case_insensitive_with_id_fallback() {
        let c = Color32::WHITE;
        let ps = vec![
            proj("b", "same", c, 0, 0),
            proj("a", "Same", c, 0, 0),
            proj("c", "alpha", c, 0, 0),
        ];
        let order = display_order(&ps, ProjectSort::ByName);
        assert_eq!(ps[order[0]].name, "alpha");
        assert_eq!(ps[order[1]].id, "a"); // "Same" == "same" без регистра → по id
        assert_eq!(ps[order[2]].id, "b");
    }

    #[test]
    fn dates_are_newest_first() {
        let c = Color32::WHITE;
        let ps = vec![
            proj("1", "old",  c, 100, 300),
            proj("2", "new",  c, 300, 100),
            proj("3", "mid",  c, 200, 200),
        ];
        let by_created  = display_order(&ps, ProjectSort::ByCreated);
        assert_eq!(names(&ps, &by_created), ["new", "mid", "old"]);
        let by_modified = display_order(&ps, ProjectSort::ByModified);
        assert_eq!(names(&ps, &by_modified), ["old", "mid", "new"]);
    }

    #[test]
    fn equal_dates_fall_back_to_name() {
        let c = Color32::WHITE;
        let ps = vec![
            proj("1", "Б", c, 0, 0),
            proj("2", "А", c, 0, 0),
        ];
        let order = display_order(&ps, ProjectSort::ByCreated);
        assert_eq!(names(&ps, &order), ["А", "Б"]);
    }

    #[test]
    fn touch_only_moves_forward() {
        let mut p = proj("1", "p", Color32::WHITE, 100, 100);
        p.touch(50);
        assert_eq!(p.last_edited, 100);
        p.touch(200);
        assert_eq!(p.last_edited, 200);
    }

    #[test]
    fn new_project_last_edited_equals_created_at() {
        let p = LoadedProject::new("1".into(), "p".into(), Color32::WHITE, 777);
        assert_eq!(p.last_edited, 777);
    }

    #[test]
    fn empty_and_single() {
        assert!(display_order(&[], ProjectSort::ByColor).is_empty());
        let ps = vec![proj("1", "p", Color32::WHITE, 0, 0)];
        assert_eq!(display_order(&ps, ProjectSort::ByModified), vec![0]);
    }
}
