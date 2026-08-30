//! Манифест проектов — `projects_index.json`.
//!
//! Лёгкая, быстро читаемая проекция всех проектов (id/name/color_hex/
//! task_count), без самих задач. Используется, чтобы отрисовать первый
//! кадр (свитчер проектов) ещё до того, как реальные файлы проектов
//! прочитаны — см. Cue_Старт_Приложения_План.txt.
//!
//! ЭТАП 1: только сама структура + чтение/запись файла. Пока НИКЕМ не
//! вызывается — ни из App::new(), ни из LoadedProject::save()/
//! delete_file(). Существующее поведение программы не меняется.
//!
//! task_count — ВСЕ задачи (main + subs), включая неактивные рутины
//! (см. обсуждение в чате — решено намеренно).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Clone, PartialEq)]
pub struct ManifestEntry {
    pub name:               String,
    pub color_hex:          String,
    pub task_count:         usize,
    pub has_active_routine: bool,
    /// Ключ сортировки для произвольного порядка проектов (v2.1). В v2
    /// хранится и синхронизируется, но нигде не используется для
    /// отображения.
    #[serde(default)]
    pub order_key:          f64,
}

/// id проекта → его запись в манифесте.
pub type Manifest = HashMap<String, ManifestEntry>;

fn manifest_path() -> std::path::PathBuf {
    super::app_dir().join("projects_index.json")
}

/// Читает манифест с диска. Отсутствующий файл, битый JSON — в обоих
/// случаях возвращает пустую карту (не Result — вызывающий код и так
/// трактует "нет записей" и "не смогли прочитать" одинаково, см. шаг
/// 3(ii) плана старта: оба случая триггерят один и тот же миграционный
/// путь).
pub fn load() -> Manifest {
    std::fs::read_to_string(manifest_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Атомарная перезапись ВСЕГО манифеста целиком (tmp + rename — тот же
/// паттерн, что и LoadedProject::save(), project.rs:157-160). Вызывающий
/// код отвечает за то, чтобы `m` содержал все нужные записи — это не
/// частичное обновление одной записи, а именно "записать вот такой,
/// целиком, манифест".
fn write_whole(m: &Manifest) {
    let dir = super::app_dir();
    let path = manifest_path();
    let tmp  = dir.join("projects_index.json.tmp");
    if let Ok(j) = serde_json::to_string(m) {
        if std::fs::write(&tmp, &j).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

/// Читает текущий манифест, добавляет/обновляет ОДНУ запись по id,
/// пишет обратно целиком. Read-modify-write — используется из
/// LoadedProject::save() (этап 2, ещё не подключено).
pub fn upsert_entry(id: &str, entry: ManifestEntry) {
    let mut m = load();
    m.insert(id.to_owned(), entry);
    write_whole(&m);
}

/// Читает текущий манифест, удаляет ОДНУ запись по id, пишет обратно
/// целиком. Используется из LoadedProject::delete_file() (этап 2, ещё
/// не подключено).
pub fn remove_entry(id: &str) {
    let mut m = load();
    if m.remove(id).is_some() {
        write_whole(&m);
    }
}

/// Строит манифест с нуля из уже загруженного полного списка проектов
/// и пишет его целиком ОДНИМ разом (не через upsert_entry на каждый —
/// это разовое действие при миграции/восстановлении, не поэлементное
/// сохранение, см. шаг 3(ii) плана старта — "Ветка А" при отсутствующем
/// или пустом манифесте).
pub fn rebuild_from(projects: &[crate::project::LoadedProject]) {
    let m: Manifest = projects.iter().map(|p| {
        let task_count = p.main.len() + p.subs.len();
        (p.id.clone(), ManifestEntry {
            name:               p.name.clone(),
            color_hex:          p.color_hex.clone(),
            task_count,
            has_active_routine: p.has_active_routine(),
            order_key:          p.order_key,
        })
    }).collect();
    write_whole(&m);
}
