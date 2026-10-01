use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use eframe::egui::Color32;
use std::sync::atomic::{AtomicU64, Ordering};
use crate::settings::{NewTaskPos, Settings};

pub fn current_time() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn gen_random_string(len: usize) -> String {
    static CTR: AtomicU64 = AtomicU64::new(0);
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    let mut n = t
        ^ ((std::process::id() as u64) << 32)
        ^ CTR.fetch_add(1, Ordering::Relaxed).wrapping_mul(0x9e3779b97f4a7c15);
    const CH: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut id = String::with_capacity(len);
    for _ in 0..len {
        id.push(CH[(n as usize) % 62] as char);
        n = n.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    }
    id
}

/// Шаг ключей после компакции — как у bootstrap (`i * 1000`).
pub const COMPACT_STEP: f64 = 1000.0;
/// Зазор между соседними order_key, ниже которого пора компактировать.
/// Запас ~4 порядка до реального предела точности f64 при ключах до 1e6.
pub const COMPACT_GAP: f64 = 1e-6;

pub fn gen_id() -> String {
    gen_random_string(12)
}

/// Id задачи: [project_id]-[4 случайных символа]-[unix-время в целых секундах].
/// Каждый слой страхует свою узкую зону: project_id — от коллизий между
/// проектами (и так уже почти невозможных, но теперь гарантированно нет),
/// ts — от коллизий внутри одного проекта, короткий рандомный хвост — только
/// от коллизии "тот же проект и та же секунда" (более 3800 задач в секунду
/// в одном проекте для 50% шанса — на практике недостижимо). Коллизия
/// требует совпадения всех трёх сразу, а не любого одного по отдельности.
pub fn gen_task_id(project_id: &str) -> String {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{project_id}-{}-{ts}", gen_random_string(4))
}

/// Настоящий случайный секрет для пейринга устройств — длиннее и отдельно
/// от gen_id() не потому что математически отличается, а чтобы явно не
/// путать "идентификатор чего-либо" с "секрет, который нельзя вычислить
/// по публичным данным" (см. обсуждение — раньше token был детерминирован
/// от двух device_id, что делало его вычислимым кем угодно в LAN).
pub fn gen_token() -> String {
    gen_random_string(24)
}

pub fn hex_to_color32(hex: &str) -> Option<Color32> {
    let h = hex.trim_start_matches('#');
    if h.len() != 6 { return None; }
    let r = u8::from_str_radix(&h[0..2], 16).ok()?;
    let g = u8::from_str_radix(&h[2..4], 16).ok()?;
    let b = u8::from_str_radix(&h[4..6], 16).ok()?;
    Some(Color32::from_rgb(r, g, b))
}

pub fn color32_to_hex(c: Color32) -> String {
    format!("#{:02X}{:02X}{:02X}", c.r(), c.g(), c.b())
}

pub fn projects_dir() -> std::path::PathBuf {
    super::app_dir().join("projects")
}

/// Цикличная рутина: задача снова становится активной через `every`
/// секунд после последнего выполнения (плавающий интервал, в отличие от
/// календарных week/month/direct). Один интервал, без списка. Отсчёт идёт
/// от `max(from, TaskData::completed_at)` в чистом UTC: `from` — точка
/// старта на момент создания рутины, дальше работает completed_at,
/// которое приходит по сети вместе с CompleteTask. Цикл с `every == 0`
/// считается отсутствующим (иначе срабатывал бы на каждом тике). Оба поля
/// с #[serde(default)] — ошибка в ручной правке JSON не должна ронять
/// разбор всей рутины (а с ней и файла проекта).
#[derive(Serialize, Deserialize, Clone, Default, PartialEq)]
pub struct Cycle {
    #[serde(default)]
    pub every: u64,
    #[serde(default)]
    pub from:  u64,
}

fn is_zero_u64(v: &u64) -> bool { *v == 0 }

/// Расписание рутины. week/month/direct — независимые опциональные списки,
/// могут присутствовать одновременно (см. Cue_Routines_Implementation_Plan.txt,
/// раздел 1 — это отличается от исходного design-дока, где был единственный
/// type). Пустой список никогда не хранится как `[]` — либо None, либо
/// непустой Vec.
#[derive(Serialize, Deserialize, Clone, Default, PartialEq)]
pub struct Routine {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub week:   Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub month:  Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direct: Option<Vec<String>>,
    /// Цикличная рутина (см. Cycle). Сосуществует с week/month/direct.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cycle:  Option<Cycle>,
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub last_triggered_at: u64,
}

impl Routine {
    /// true, если во всех трёх списках пусто (значит рутину пора убрать
    /// целиком — см. раздел 6.2 плана: сохранение с пустым редактором = None).
    pub fn is_empty(&self) -> bool {
        self.week.as_ref().map_or(true, |v| v.is_empty())
            && self.month.as_ref().map_or(true, |v| v.is_empty())
            && self.direct.as_ref().map_or(true, |v| v.is_empty())
            && self.cycle.as_ref().map_or(true, |c| c.every == 0)
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct TaskData {
    pub text:       String,
    #[serde(default)]
    pub routine:    Option<Routine>,
    pub created_at: u64,
    #[serde(default)]
    pub order_key:  f64,
    #[serde(default)]
    pub text_edited_at:    u64,
    #[serde(default)]
    pub routine_edited_at: u64,
    /// LWW-метка для MoveTask — только приёмная сторона в v2 (UI drag &
    /// drop локальный, без опа), появится в v2.1.
    #[serde(default)]
    pub pos_edited_at:     u64,
    /// Время последнего переноса задачи между проектами (TransferTask,
    /// v2.1) — защищает от гонки "два устройства решили увезти в разные
    /// места одновременно": выигрывает перенос с более поздним ts. Здесь
    /// не в оплоге, а прямо на задаче — переезжает вместе с ней, где бы
    /// она сейчас ни лежала. Приёмная сторона в v2 заранее, как и с
    /// pos_edited_at выше — отправки самого TransferTask ещё нет.
    #[serde(default)]
    pub transferred_at:    u64,
    /// Время (UTC, из op.ts) последнего выполнения задачи. Хранится на
    /// самой задаче, а не в Routine: выполнение может прийти раньше, чем
    /// SetRoutine, который впервые вешает на задачу рутину, — тогда ему
    /// больше некуда лечь. Нужно Cycle как база отсчёта; у обычных задач
    /// всегда 0 (их выполнение удаляет) и в JSON не пишется.
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub completed_at:      u64,
}

/// Эффективно активна ли задача (для сортировки/выбора следующей main).
/// Обычная задача (без рутины) всегда считается активной.
pub fn is_active_task(t: &TaskData) -> bool {
    t.routine.as_ref().map_or(true, |r| r.active)
}

#[derive(Serialize, Deserialize)]
struct TasksFile {
    main: IndexMap<String, TaskData>,
    subs: IndexMap<String, TaskData>,
}

#[derive(Serialize, Deserialize)]
struct ProjectFile {
    ver:   u32,
    name:  String,
    color: String,
    tasks: TasksFile,
    #[serde(default)]
    last_edited: u64,
    #[serde(default)]
    created_at:  u64,
    #[serde(default)]
    main_edited_at: u64,
    #[serde(default)]
    name_edited_at:  u64,
    #[serde(default)]
    color_edited_at: u64,
    /// Ключ сортировки для произвольного порядка проектов (v2.1).
    #[serde(default)]
    order_key:       f64,
    #[serde(default)]
    order_key_edited_at: u64,
}

pub struct LoadedProject {
    pub id:         String,
    pub name:       String,
    pub color:      Color32,
    pub main:       IndexMap<String, TaskData>,
    pub subs:       IndexMap<String, TaskData>,
    pub color_hex:  String,
    pub created_at: u64,
    /// Время последнего изменения проекта (UTC, секунды) — для сортировки
    /// списка проектов «по дате изменения». Двигается ТОЛЬКО через
    /// `touch()` (действия с задачами, rename/recolor), а не каждым
    /// `save()`: тик рутин, компакция и бутстрап — не правка пользователя.
    pub last_edited: u64,
    /// true — реальные данные с диска. false — манифестная заглушка
    /// (Ветка Б холодного старта, main.rs), main/subs пока пусты не
    /// потому что проект пуст, а потому что его ещё не прочитали.
    /// Этап 6 (мёрж батча от потока-загрузчика) использует это поле,
    /// чтобы решить — доверять батчу целиком (false) или скипнуть
    /// (true, уже реальные данные).
    pub loaded:     bool,
    /// LWW-регистр "кто занимает main" — конкурируют PromoteTask и
    /// AddTask{target: Main}, а также авто-заполнение main внутри
    /// complete_task (см. apply_promote_task/apply_add_to_main ниже).
    pub main_edited_at: u64,
    /// LWW-метки для RenameProject/RecolorProject — только приёмная сторона
    /// в v2 (UI-отправки нет, появится в v2.1), нужны заранее ради
    /// обратной совместимости: v2 уже должна уметь корректно принять такой
    /// оп от будущего v2.1-устройства.
    pub name_edited_at:  u64,
    pub color_edited_at: u64,
    /// Ключ сортировки для произвольного порядка проектов (v2.1). В v2
    /// хранится и синхронизируется (см. MoveProject в apply.rs), но нигде
    /// не используется для отображения — сортировки "Произвольный" в v2 нет.
    pub order_key:           f64,
    pub order_key_edited_at: u64,
    /// Кэш числа задач (main+subs) — для отрисовки в свитчере проектов, в
    /// т.ч. пока проект ещё манифестная заглушка (loaded: false) и main/subs
    /// физически пусты. НЕ часть ProjectFile — считается заново при каждом
    /// save() и при загрузке, здесь только чтобы не открывать файл лишний
    /// раз ради одной цифры.
    pub task_count: usize,
}

impl LoadedProject {
    /// Единая точка применения правки текста задачи — и для локального
    /// действия пользователя, и для входящего сетевого опа. LWW по `ts`:
    /// применяется, только если `ts` строго новее уже сохранённого
    /// `text_edited_at`, иначе тихо игнорируется (более старая правка,
    /// пришедшая с опозданием, не должна затирать более свежую).
    pub fn apply_edit_task(&mut self, task_id: &str, text: &str, ts: u64) -> bool {
        let Some(t) = self.main.get_mut(task_id).or_else(|| self.subs.get_mut(task_id)) else {
            return false;
        };
        if ts <= t.text_edited_at { return false; }
        t.text = text.to_owned();
        t.text_edited_at = ts;
        true
    }

    /// Единая точка применения перемещения задачи (drag & drop списка) — и
    /// для локального действия пользователя, и для входящего сетевого опа.
    /// LWW по `ts` на `pos_edited_at`, как и у остальных apply_*.
    ///
    /// В отличие от `reorder_sub` (локальный драг — знает физический индекс
    /// места сброса из позиции курсора) здесь известен только конечный
    /// `order_key`: устройство, принимающее оп, само находит, между какими
    /// соседями по `order_key` физически встанет задача, и переставляет её
    /// в `subs`. Физическая позиция таким образом никогда не расходится с
    /// `order_key` ни на одном устройстве — это важно, потому что при
    /// выключенном тумблере "неактивные рутины в конец" для отображения
    /// используется именно физический порядок, не `order_key` напрямую.
    ///
    /// `main` — фиксированный слот без понятия физической позиции: для неё
    /// перестановка не нужна, только обновление ключа и метки LWW.
    pub fn apply_move_task(&mut self, task_id: &str, order_key: f64, ts: u64) -> bool {
        if let Some(t) = self.main.get_mut(task_id) {
            if ts <= t.pos_edited_at { return false; }
            t.order_key     = order_key;
            t.pos_edited_at = ts;
            return true;
        }

        let Some(idx) = self.subs.get_index_of(task_id) else { return false; };
        let Some((_, t)) = self.subs.get_index(idx) else { return false; };
        if ts <= t.pos_edited_at { return false; }

        let Some((id, mut task)) = self.subs.shift_remove_index(idx) else { return false; };
        task.order_key     = order_key;
        task.pos_edited_at = ts;

        let target = self.subs.iter()
            .position(|(_, t)| t.order_key > order_key)
            .unwrap_or(self.subs.len());
        self.subs.shift_insert(target, id, task);
        true
    }

    /// Перенумерация order_key всех задач subs одним махом (см.
    /// OpKind::CompactOrder). Задача `order[i]` получает ключ `i * 1000`;
    /// ранги считаются по ВСЕМУ списку отправителя, включая id, которых у
    /// нас нет (не долетели/удалены/уехали) — они просто пропускаются, но
    /// ранг не сдвигают. Задачи, которых нет в списке, не трогаем.
    ///
    /// LWW по каждой задаче: пропускаем только если у неё метка СТРОГО
    /// новее (`ts < pos_edited_at`) — её двигали после этой компакции, и
    /// чужой старый список перебивать нельзя. Равенство проходит
    /// сознательно: метки в секундах, и перетаскивание с компакцией сразу
    /// за ним имеют один и тот же ts — иначе пропустили бы как раз только
    /// что перетащенную задачу. Метку не понижаем (`max`), иначе ранее
    /// отвергнутый старый MoveTask мог бы внезапно пройти.
    ///
    /// В конце — стабильная сортировка subs по ключу: весь код (reorder_sub,
    /// apply_move_task) опирается на инвариант "физический порядок subs =
    /// порядок по order_key". Возвращает true, если что-то изменилось.
    pub fn apply_compact_order(&mut self, order: &[String], ts: u64) -> bool {
        let mut changed = false;
        for (rank, id) in order.iter().enumerate() {
            let Some(task) = self.subs.get_mut(id.as_str()) else { continue; };
            if ts < task.pos_edited_at { continue; }
            task.order_key     = rank as f64 * COMPACT_STEP;
            task.pos_edited_at = task.pos_edited_at.max(ts);
            changed = true;
        }
        if changed {
            self.subs.sort_by(|_, a, _, b| a.order_key.total_cmp(&b.order_key));
        }
        changed
    }

    /// Локальная компакция: порядок берём из физического порядка subs
    /// (он же — порядок на экране при выключенной группировке), применяем
    /// к себе ТОЙ ЖЕ функцией, что и получатели, и отдаём список для опа.
    pub fn compact_order(&mut self, ts: u64) -> Vec<String> {
        let order: Vec<String> = self.subs.keys().cloned().collect();
        self.apply_compact_order(&order, ts);
        order
    }

    /// true, если между какими-то соседними задачами subs зазор по
    /// order_key меньше порога (или ключи равны/идут не по порядку) —
    /// пора слать CompactOrder. Сужает зазоры только reorder_sub (середина
    /// между соседями), но проверка дёшева, поэтому её зовут и после
    /// добавления задачи.
    pub fn needs_compaction(&self) -> bool {
        self.subs.values().zip(self.subs.values().skip(1))
            .any(|(a, b)| b.order_key - a.order_key < COMPACT_GAP)
    }

    /// Единая точка применения нового расписания рутины — и для локальной
    /// правки в редакторе, и для входящего сетевого опа. LWW по `ts` на
    /// сам факт правки расписания (`routine_edited_at`), но `active`/
    /// `last_triggered_at` НИКОГДА не берутся из входящих данных напрямую —
    /// это локально вычисляемая производная: подставляем уже имеющийся
    /// локальный `last_triggered_at` поверх присланного расписания и сразу
    /// пересчитываем due, чтобы не открыть заново уже обработанные локально
    /// вхождения и не разминуться с уведомлением при активации.
    /// LWW-регистр "кто занимает main". Применяется, только если `ts`
    /// строго новее уже сохранённого `main_edited_at` — иначе более старый,
    /// задержавшийся в пути промоушен не должен вытеснять то, что уже
    /// заняло слот позже по факту (см. обсуждение гонки PromoteTask/
    /// CompleteTask). Общий метод для локального клика и входящего опа.
    pub fn apply_promote_task(&mut self, task_id: &str, ts: u64) -> bool {
        if ts <= self.main_edited_at { return false; }
        let Some(i) = self.subs.get_index_of(task_id) else { return false; };
        self.promote_sub(i);
        self.main_edited_at = ts;
        true
    }

    /// То же самое для AddTask{target: Main} — но здесь задача создаётся
    /// с нуля, поэтому проигрыш гонки за main не должен приводить к потере
    /// самой задачи: она просто уходит в конец subs вместо main.
    pub fn apply_add_to_main(&mut self, task_id: String, mut task: TaskData, ts: u64) {
        if ts > self.main_edited_at {
            if !self.main.is_empty() {
                let (old_id, mut old) = self.main.shift_remove_index(0).unwrap();
                old.order_key = self.next_end_key();
                self.subs.insert(old_id, old);
            }
            self.main.insert(task_id, task);
            self.main_edited_at = ts;
        } else {
            task.order_key = self.next_end_key();
            self.subs.insert(task_id, task);
        }
    }

    pub fn apply_set_routine(&mut self, task_id: &str, mut routine: Option<Routine>, ts: u64) -> bool {
        let Some(t) = self.main.get_mut(task_id).or_else(|| self.subs.get_mut(task_id)) else {
            return false;
        };
        if ts <= t.routine_edited_at { return false; }
        t.routine_edited_at = ts;

        if let Some(r) = routine.as_mut() {
            // Тройной фолбэк: своя история (если задача уже существовала
            // локально) → присланная в опе (bootstrap с реальным устройством-
            // источником) → ноль (совсем новая задача без истории вообще).
            // Не путать с "доверять чужому active/last_triggered_at
            // напрямую" — due всё равно пересчитывается заново ниже, это
            // только выбор точки отсчёта.
            r.last_triggered_at = t.routine.as_ref().map(|old| old.last_triggered_at)
                .unwrap_or(r.last_triggered_at);
            let now = crate::routine_scheduler::local_now();
            let utc = current_time();
            if let Some(ago) = crate::routine_scheduler::due_secs_ago(r, t.completed_at, now, utc) {
                r.active = true;
                r.last_triggered_at = now;
                crate::routine_scheduler::prune_expired_direct(r, now);
                if ago <= crate::routine_scheduler::NOTIFY_WINDOW_SECS {
                    crate::notify::send(&t.text, &self.name, &self.color_hex);
                }
            } else {
                r.active = false;
            }
        }
        t.routine = routine;
        true
    }

    pub fn has_active_routine(&self) -> bool {
        self.main.values().chain(self.subs.values())
            .any(|t| t.routine.as_ref().is_some_and(|r| r.active))
    }
    pub fn new(id: String, name: String, color: Color32, created_at: u64) -> Self {
        Self {
            color_hex: color32_to_hex(color),
            id, name, color,
            main:       IndexMap::new(),
            subs:       IndexMap::new(),
            created_at,
            last_edited: created_at,
            loaded:     true,
            main_edited_at: 0, name_edited_at: 0, color_edited_at: 0,
            order_key: 0.0, order_key_edited_at: 0,
            task_count: 0,
        }
    }

    /// Отмечает, что проект изменён в момент `ts`. Только вперёд (`max`):
    /// запоздавший оп со старым ts не должен откатывать дату назад.
    pub fn touch(&mut self, ts: u64) {
        self.last_edited = self.last_edited.max(ts);
    }

    pub fn main_text(&self) -> Option<&str> {
        self.main.values().next().map(|t| t.text.as_str())
    }

    pub fn save(&mut self) {
        self.task_count = self.main.len() + self.subs.len();
        let file = ProjectFile {
            ver:   1,
            name:  self.name.clone(),
            color: self.color_hex.clone(),
            tasks: TasksFile {
                main: self.main.clone(),
                subs: self.subs.clone(),
            },
            last_edited: self.last_edited,
            created_at:  self.created_at,
            main_edited_at: self.main_edited_at, name_edited_at: self.name_edited_at, color_edited_at: self.color_edited_at,
            order_key: self.order_key, order_key_edited_at: self.order_key_edited_at,
        };
        let path = projects_dir().join(format!("{}.json", self.id));
        let tmp  = projects_dir().join(format!("{}.json.tmp", self.id));
        if let Ok(j) = serde_json::to_string(&file) {
            if std::fs::write(&tmp, &j).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
        // Манифест — ПОСЛЕ файла проекта, всегда следующим шагом. Он может
        // только отставать от истины (при краше между двумя записями), но
        // никогда не должен опережать её. См. Cue_Мёрж_Батча_И_Битые_Файлы.txt.
        crate::manifest::upsert_entry(&self.id, crate::manifest::ManifestEntry {
            name:               self.name.clone(),
            color_hex:          self.color_hex.clone(),
            task_count:         self.task_count,
            has_active_routine: self.has_active_routine(),
            order_key:          self.order_key,
            created_at:         self.created_at,
            last_edited:        self.last_edited,
        });
    }



    pub(crate) fn next_end_key(&self) -> f64 {
        self.subs.values().map(|t| t.order_key)
            .reduce(f64::max).map_or(0.0, |m| m + 1000.0)
    }
    pub(crate) fn next_beg_key(&self) -> f64 {
        self.subs.values().map(|t| t.order_key)
            .reduce(f64::min).map_or(0.0, |m| m - 1000.0)
    }

    pub fn delete_file(&self) {
        let path = projects_dir().join(format!("{}.json", self.id));
        let _    = std::fs::remove_file(path);
        crate::manifest::remove_entry(&self.id);
    }

    pub fn add_task(&mut self, id: String, text: String, s: &Settings) {
        let mut task = TaskData {
            text, routine: None, created_at: current_time(), order_key: 0.0,
            text_edited_at: current_time(), routine_edited_at: 0, pos_edited_at: 0,
            transferred_at: 0, completed_at: 0,
        };

        if self.main.is_empty() {
            self.main.insert(id, task);
            self.main_edited_at = current_time();
            return;
        }
        if s.replace_main {
            let (old_id, mut old_task) = self.main.shift_remove_index(0).unwrap();
            self.main.insert(id, task);
            self.main_edited_at = current_time();
            match s.new_task_pos {
                NewTaskPos::End => {
                    old_task.order_key = self.next_end_key();
                    self.subs.insert(old_id, old_task); // insert() = физический конец для новых ключей
                }
                NewTaskPos::Beginning => {
                    old_task.order_key = self.next_beg_key();
                    self.subs.shift_insert(0, old_id, old_task);
                }
            }
        } else {
            match s.new_task_pos {
                NewTaskPos::End => {
                    task.order_key = self.next_end_key();
                    self.subs.insert(id, task); // insert() = физический конец для новых ключей
                }
                NewTaskPos::Beginning => {
                    task.order_key = self.next_beg_key();
                    self.subs.shift_insert(0, id, task);
                }
            }
        }
    }

    /// Единая точка завершения задачи — не важно, откуда пришёл вызов:
    /// клик по main-слоту или крестик по активной рутине в subs. Ищет
    /// `task_id` сначала в main, потом в subs, применяет поведение по месту
    /// находки. Обычная задача (без рутины) — удаляется. Задача с рутиной
    /// НИКОГДА не удаляется отсюда: рутина гасится (active=false), а если
    /// она исчерпана (больше не сработает никогда) — снимается совсем, и
    /// задача остаётся обычной. Удалять исчерпанную задачу или нет — решает
    /// не эта функция, а отправитель: если включена настройка
    /// `delete_spent_routines`, он следом шлёт обычный DeleteTask.
    ///
    /// `now` — момент выполнения в локальном базисе (для prune direct-дат):
    /// для локального клика routine_scheduler::local_now(), для входящего
    /// по сети опа — op.ts. `ts` — UTC-метка действия (current_time() либо
    /// op.ts): идёт в main_edited_at и в LWW-гейт снятия рутины.
    ///
    /// Возвращает `None`, если задачи нет; `Some(true)`, если рутина была
    /// исчерпана и снята (задача осталась обычной); иначе `Some(false)`.
    ///
    /// Если задача уже в subs — order_key и физическую позицию НЕ трогаем
    /// (мутация на месте). Из main выход в subs — всегда новая запись в
    /// конец (позиции раньше не было, терять нечего).
    ///
    /// `sender_routine_ts` — `routine_edited_at` отправителя, если у его
    /// задачи в момент выполнения была рутина (иначе 0; локально всегда 0).
    /// Если он новее моего — у меня расписание устарело, SetRoutine ещё в
    /// пути: задачу нельзя считать обычной и удалять, она ведётся как
    /// рутинная (уходит в subs), а время выполнения запоминается в
    /// `completed_at` — расписание, пришедшее позже, посчитает от него.
    pub fn complete_task(&mut self, task_id: &str, now: u64, ts: u64, sender_routine_ts: u64) -> Option<bool> {
        if self.main.contains_key(task_id) {
            let (id, mut task) = self.main.shift_remove_entry(task_id).unwrap();
            let had_routine = Self::counts_as_routine(&task, sender_routine_ts);
            let spent = Self::settle_routine(&mut task, now, ts);
            if had_routine { task.completed_at = task.completed_at.max(ts); }

            // Продвигаем в main первую ЭФФЕКТИВНО АКТИВНУЮ sub-задачу. Это
            // ДО вставки остатка завершённой задачи: обычная задача (у
            // которой рутину только что сняли) считается активной и иначе
            // тут же вернулась бы в main.
            if let Some(pos) = self.subs.iter().position(|(_, t)| is_active_task(t)) {
                let (next_id, next_task) = self.subs.shift_remove_index(pos).unwrap();
                self.main.insert(next_id, next_task);
                self.main_edited_at = ts; // UTC, не local_now — тот же базис, что у Promote/AddTask
            }

            if had_routine {
                task.order_key = self.next_end_key();
                self.subs.insert(id, task); // физический конец; display-порядок группирует отдельно
            }
            // had_routine == false — обычная задача, просто пропадает
            return Some(spent);
        }

        let task = self.subs.get_mut(task_id)?;
        if !Self::counts_as_routine(task, sender_routine_ts) {
            self.subs.shift_remove(task_id); // обычная задача в subs — просто удаляется
            return Some(false);
        }
        let spent = Self::settle_routine(task, now, ts);
        task.completed_at = task.completed_at.max(ts);
        Some(spent)
    }

    /// Вести ли выполнение этой задачи как рутинной: рутина есть у нас, либо
    /// отправитель знает расписание новее нашего (см. complete_task).
    fn counts_as_routine(task: &TaskData, sender_routine_ts: u64) -> bool {
        task.routine.is_some()
            || (sender_routine_ts > 0 && sender_routine_ts > task.routine_edited_at)
    }

    /// Гасит рутину завершённой задачи; если после чистки прошедших
    /// direct-дат она больше не сработает никогда — снимает её совсем.
    /// Снятие под LWW-гейтом по routine_edited_at: если расписание
    /// правили позже этого завершения (пока оп летел), чужую правку не
    /// затираем — только гасим. Возвращает true, если рутина снята.
    fn settle_routine(task: &mut TaskData, now: u64, ts: u64) -> bool {
        let Some(routine) = task.routine.as_mut() else { return false; };
        crate::routine_scheduler::prune_expired_direct(routine, now);
        routine.active = false;
        if routine.is_empty() && ts >= task.routine_edited_at {
            task.routine = None;
            task.routine_edited_at = ts;
            return true;
        }
        false
    }

    pub fn promote_sub(&mut self, i: usize) {
        let (sub_id, sub_task) = self.subs.shift_remove_index(i).unwrap();
        if !self.main.is_empty() {
            let (old_id, mut old_task) = self.main.shift_remove_index(0).unwrap();
            old_task.order_key = self.next_beg_key();
            self.subs.shift_insert(0, old_id, old_task);
        }
        self.main.insert(sub_id, sub_task);
    }

    pub fn delete_sub(&mut self, i: usize) {
        self.subs.shift_remove_index(i);
    }

    /// Инлайн-редактирование текста sub-задачи (кнопка-карандаш).
    pub fn edit_sub(&mut self, i: usize, text: String) {
        if let Some((_, task)) = self.subs.get_index_mut(i) {
            task.text = text;
        }
    }

    /// Физическое перемещение задачи в subs по локальному действию юзера
    /// (drag & drop). `from` — физический индекс перетаскиваемой задачи ДО
    /// перемещения. `to_before` — физический индекс задачи, ПЕРЕД которой
    /// нужно вставить перетаскиваемую (в display-порядке на момент
    /// отпускания мыши); `None` — вставить в самый конец списка.
    ///
    /// order_key пересчитывается интерполяцией между order_key НОВЫХ
    /// физических соседей (после перемещения) — сознательно не учитывает
    /// группировку active/inactive: это просто число между двумя другими
    /// числами, коллизии с order_key задач из другой группы безобидны и
    /// максимум приводят к сдвигу на одну строку при реактивации рутины в
    /// редком случае.
    ///
    /// Физическая позиция в IndexMap двигается вместе с order_key, чтобы
    /// порядок был согласован и при выключенном тумблере группировки (где
    /// физический порядок — это и есть порядок отображения).
    ///
    /// Возвращает `(task_id, новый order_key)` для записи MoveTask-опа —
    /// `None`, если дропнули на себя же (no-op).
    pub fn reorder_sub(&mut self, from: usize, to_before: Option<usize>, ts: u64) -> Option<(String, f64)> {
        if Some(from) == to_before { return None; } // дропнули на себя же — no-op
        let (id, mut task) = self.subs.shift_remove_index(from)?;

        // to_before был индексом ДО удаления; если он шёл после удалённого
        // слота — после shift_remove_index он сместился на 1 назад.
        let target = match to_before {
            None => self.subs.len(), // конец списка
            Some(before) if before > from => before - 1,
            Some(before) => before,
        };

        let prev_key = target.checked_sub(1)
            .and_then(|i| self.subs.get_index(i))
            .map(|(_, t)| t.order_key);
        let next_key = self.subs.get_index(target).map(|(_, t)| t.order_key);
        task.order_key = match (prev_key, next_key) {
            (Some(p), Some(n)) => (p + n) / 2.0,
            (Some(p), None)    => p + 1000.0,
            (None, Some(n))    => n - 1000.0,
            (None, None)       => 0.0,
        };
        task.pos_edited_at = ts;

        let order_key = task.order_key;
        self.subs.shift_insert(target, id.clone(), task);
        Some((id, order_key))
    }

}

/// Синхронно читает и парсит ОДИН файл проекта по id. None — файл
/// отсутствует, не читается, бьётся при парсинге, либо цвет невалиден.
/// Тот же путь парсинга, что и в load_all_projects() — вынесен отдельно,
/// чтобы переиспользовать и здесь, и при клике на непрогруженный проект
/// (Этап 8 плана), и в фолбэке ниже.
pub fn load_one(id: &str) -> Option<LoadedProject> {
    let path = projects_dir().join(format!("{id}.json"));
    let text = std::fs::read_to_string(&path).ok()?;
    let file: ProjectFile = serde_json::from_str(&text).ok()?;
    let color = hex_to_color32(&file.color)?;
    let task_count = file.tasks.main.len() + file.tasks.subs.len();
    Some(LoadedProject {
        id: id.to_owned(),
        name:       file.name,
        color,
        color_hex:  file.color,
        main:       file.tasks.main,
        subs:       file.tasks.subs,
        created_at: file.created_at,
        last_edited: file.last_edited.max(file.created_at),
        loaded:     true,
        main_edited_at: file.main_edited_at, name_edited_at: file.name_edited_at, color_edited_at: file.color_edited_at,
        order_key: file.order_key, order_key_edited_at: file.order_key_edited_at,
        task_count,
    })
}

/// Сколько проектов пробуем прочитать на старте, прежде чем сдаться и
/// создать дефолтный. Ограничено намеренно — при системной порче диска
/// неограниченная цепочка попыток могла бы затянуть старт на
/// неопределённое время, см. Cue_Мёрж_Батча_И_Битые_Файлы.txt,
/// "СЦЕНАРИЙ: АКТИВНЫЙ ПРОЕКТ БИТ/НЕДОСТУПЕН ИМЕННО НА СТАРТЕ".
const STARTUP_LOAD_ATTEMPTS: usize = 3;

/// Синхронно загружает активный проект на старте, с фолбэком для
/// битого/отсутствующего файла. `preferred_id` — id, который юзер хочет
/// видеть активным (последний открытый либо зафиксированный в настройках,
/// см. Settings::preferred_project_id). Остальные кандидаты берутся из
/// манифеста по order_key, пока не наберётся STARTUP_LOAD_ATTEMPTS попыток
/// или кандидаты не кончатся; если ни один не прочитался — дефолтный Cue.
pub fn load_active_with_fallback(
    manifest:     &crate::manifest::Manifest,
    preferred_id: Option<&str>,
) -> LoadedProject {
    let mut rest: Vec<&str> = manifest.keys()
        .map(String::as_str)
        .filter(|id| Some(*id) != preferred_id)
        .collect();
    rest.sort_by(|a, b| manifest[*a].order_key.partial_cmp(&manifest[*b].order_key)
        .unwrap_or(std::cmp::Ordering::Equal));

    preferred_id.into_iter().chain(rest)
        .take(STARTUP_LOAD_ATTEMPTS)
        .find_map(load_one)
        .unwrap_or_else(create_default_project)
}

/// Индекс активного проекта среди уже загруженных `projects` — либо
/// предпочитаемый id из настроек (Settings::preferred_project_id), либо
/// индекс 0. `None` только если `projects` пуст; создание дефолтного
/// проекта в этом случае остаётся на вызывающей стороне.
pub fn resolve_active_project(projects: &[LoadedProject], settings: &crate::settings::Settings) -> Option<usize> {
    if projects.is_empty() { return None; }
    Some(settings.preferred_project_id()
        .and_then(|id| projects.iter().position(|p| p.id == id))
        .unwrap_or(0))
}

pub fn load_all_projects() -> Vec<LoadedProject> {
    let Ok(entries) = std::fs::read_dir(projects_dir()) else { return vec![]; };

    entries.flatten()
        .filter_map(|e| {
            let path = e.path();
            let fname = path.file_name()?.to_str()?;
            if !fname.ends_with(".json") || fname.ends_with(".json.tmp") { return None; }
            let id   = path.file_stem()?.to_str()?.to_owned();
            let text = std::fs::read_to_string(&path).ok()?;
            let file: ProjectFile = serde_json::from_str(&text).ok()?;
            let color = hex_to_color32(&file.color)?;
            let task_count = file.tasks.main.len() + file.tasks.subs.len();
            let proj = LoadedProject {
                id,
                name:       file.name,
                color,
                color_hex:  file.color,
                main:       file.tasks.main,
                subs:       file.tasks.subs,
                created_at: file.created_at,
                last_edited: file.last_edited.max(file.created_at),
                loaded:     true,
                main_edited_at: file.main_edited_at, name_edited_at: file.name_edited_at, color_edited_at: file.color_edited_at,
                order_key: file.order_key, order_key_edited_at: file.order_key_edited_at,
                task_count,
            };
            // Физический порядок subs больше не пересортировывается —
            // хранится как в файле. Группировка active/inactive для показа
            // (и её сортировка order_key/created_at внутри группы) считается
            // на лету при отрисовке, см. main.rs.
            Some(proj)
        })
        .collect()
}

pub fn create_default_project() -> LoadedProject {
    let _ = std::fs::create_dir_all(projects_dir());
    let mut proj = LoadedProject::new(
        gen_id(),
        "Cue".to_owned(),
        Color32::from_rgb(74, 144, 217),
        current_time(),
    );
    proj.save();
    proj
}

#[cfg(test)]
mod compact_tests {
    use super::*;

    fn task(key: f64, stamp: u64) -> TaskData {
        TaskData {
            text: String::new(), routine: None, created_at: 0, order_key: key,
            text_edited_at: 0, routine_edited_at: 0, pos_edited_at: stamp,
            transferred_at: 0, completed_at: 0,
        }
    }

    /// Проект с subs из (id, ключ, метка) в заданном физическом порядке.
    fn proj(subs: &[(&str, f64, u64)]) -> LoadedProject {
        let mut p = LoadedProject::new("p".into(), "p".into(), Color32::WHITE, 0);
        for (id, key, stamp) in subs {
            p.subs.insert((*id).to_string(), task(*key, *stamp));
        }
        p
    }

    fn ids(p: &LoadedProject) -> Vec<&str> { p.subs.keys().map(String::as_str).collect() }
    fn keys(p: &LoadedProject) -> Vec<f64> { p.subs.values().map(|t| t.order_key).collect() }
    fn order(v: &[&str]) -> Vec<String> { v.iter().map(|s| s.to_string()).collect() }

    #[test]
    fn needs_compaction_detects_narrow_equal_and_unsorted() {
        assert!(!proj(&[]).needs_compaction());
        assert!(!proj(&[("a", 5.0, 0)]).needs_compaction());
        assert!(!proj(&[("a", 0.0, 0), ("b", 1000.0, 0), ("c", 2000.0, 0)]).needs_compaction());
        assert!(proj(&[("a", 0.0, 0), ("b", 1e-7, 0)]).needs_compaction());   // узкий зазор
        assert!(proj(&[("a", 7.0, 0), ("b", 7.0, 0)]).needs_compaction());    // ничья
        assert!(proj(&[("a", 9.0, 0), ("b", 1.0, 0)]).needs_compaction());    // не по порядку
        // отрицательные ключи (next_beg_key) — нормальный случай
        assert!(!proj(&[("a", -2000.0, 0), ("b", -1000.0, 0), ("c", 0.0, 0)]).needs_compaction());
    }

    #[test]
    fn compact_assigns_ranks_stamps_and_sorts() {
        let mut p = proj(&[("a", 10.5, 1), ("b", 10.6, 1), ("c", 99.0, 1)]);
        assert!(p.apply_compact_order(&order(&["c", "a", "b"]), 50));
        assert_eq!(ids(&p), ["c", "a", "b"]);
        assert_eq!(keys(&p), [0.0, 1000.0, 2000.0]);
        assert!(p.subs.values().all(|t| t.pos_edited_at == 50));
    }

    #[test]
    fn compact_respects_lww_strictly_newer_skipped_equal_applied() {
        // b двигали позже компакции (метка 60 > 50) — не трогаем; c ровно в
        // ту же секунду (метка 50 == 50) — применяем; метку не понижаем.
        let mut p = proj(&[("a", 1.0, 10), ("b", 2.0, 60), ("c", 3.0, 50)]);
        p.apply_compact_order(&order(&["a", "b", "c"]), 50);
        let b = &p.subs["b"];
        assert_eq!((b.order_key, b.pos_edited_at), (2.0, 60));
        let c = &p.subs["c"];
        assert_eq!((c.order_key, c.pos_edited_at), (2000.0, 50));
        assert_eq!(p.subs["a"].order_key, 0.0);
    }

    #[test]
    fn compact_unknown_ids_keep_rank_and_unlisted_untouched() {
        // у получателя нет "ghost" (не долетел) — ранг d всё равно 3000, как у
        // отправителя; "extra" не в списке — ключ и метка прежние.
        let mut p = proj(&[("a", 0.3, 0), ("b", 0.4, 0), ("d", 0.5, 0), ("extra", 7777.0, 3)]);
        p.apply_compact_order(&order(&["a", "b", "ghost", "d"]), 9);
        assert_eq!(p.subs["a"].order_key, 0.0);
        assert_eq!(p.subs["b"].order_key, 1000.0);
        assert_eq!(p.subs["d"].order_key, 3000.0);
        assert_eq!((p.subs["extra"].order_key, p.subs["extra"].pos_edited_at), (7777.0, 3));
        assert_eq!(ids(&p), ["a", "b", "d", "extra"]);
    }

    #[test]
    fn compact_is_idempotent() {
        let mut p = proj(&[("a", 0.1, 0), ("b", 0.2, 0), ("c", 0.3, 0)]);
        p.apply_compact_order(&order(&["a", "b", "c"]), 5);
        let (k1, i1) = (keys(&p), ids(&p).iter().map(|s| s.to_string()).collect::<Vec<_>>());
        p.apply_compact_order(&order(&["a", "b", "c"]), 5);
        assert_eq!(keys(&p), k1);
        assert_eq!(ids(&p), i1.iter().map(String::as_str).collect::<Vec<_>>());
    }

    #[test]
    fn compact_ignores_main_and_empty_list() {
        let mut p = proj(&[("a", 0.1, 0)]);
        p.main.insert("m".into(), task(0.5, 0));
        assert!(!p.apply_compact_order(&order(&["m", "zzz"]), 5)); // main не трогаем, zzz нет
        assert_eq!(p.main["m"].order_key, 0.5);
        assert!(!p.apply_compact_order(&[], 5));
    }

    /// Реальный сценарий: каждый раз ставим задачу сразу после первой — зазор
    /// делится пополам. Проверяем, что needs_compaction срабатывает ДО потери
    /// точности, а компакция сохраняет порядок и восстанавливает зазоры.
    #[test]
    fn repeated_drag_into_same_gap_triggers_compaction_before_ties() {
        let mut p = proj(&[("a", 0.0, 0), ("b", 1000.0, 0)]);
        for i in 0..60 { p.subs.insert(format!("x{i}"), task(2000.0 + i as f64 * 1000.0, 0)); }

        let mut compacted_at = None;
        for i in 0..60 {
            let from = p.subs.get_index_of(format!("x{i}").as_str()).unwrap();
            // перед элементом с индексом 1 (то есть сразу после "a")
            p.reorder_sub(from, Some(1), 100 + i as u64).unwrap();
            // ключи ни разу не должны слипнуться ДО проверки
            let ks = keys(&p);
            assert!(ks.windows(2).all(|w| w[0] < w[1]), "ничья/инверсия на шаге {i}: {ks:?}");
            if p.needs_compaction() {
                let before: Vec<String> = p.subs.keys().cloned().collect();
                let sent = p.compact_order(100 + i as u64);
                assert_eq!(sent, before, "компакция не должна менять физический порядок");
                assert_eq!(ids(&p), before.iter().map(String::as_str).collect::<Vec<_>>());
                assert!(!p.needs_compaction());
                assert_eq!(keys(&p)[1], 1000.0);
                compacted_at = Some(i);
                break;
            }
        }
        let at = compacted_at.expect("компакция так и не сработала");
        assert!((20..=40).contains(&at), "сработала на шаге {at}, ожидалось ~30");
    }

    /// Два устройства: отправитель делает перетаскивание + компакцию,
    /// получатель применяет MoveTask и CompactOrder в любом порядке — итог
    /// один и тот же, а запоздавший старый MoveTask отвергается.
    #[test]
    fn sender_and_receiver_converge_in_any_arrival_order() {
        let base = [("a", 0.0, 1), ("b", 1e-9, 1), ("c", 2e-9, 1), ("d", 3e-9, 1)];
        let mut sender = proj(&base);
        let mut recv_ab = proj(&base);
        let mut recv_ba = proj(&base);

        let ts = 500;
        let (moved, key) = sender.reorder_sub(3, Some(1), ts).unwrap(); // d между a и b
        assert!(sender.needs_compaction());
        let list = sender.compact_order(ts);

        // получатель 1: MoveTask, потом CompactOrder
        recv_ab.apply_move_task(&moved, key, ts);
        recv_ab.apply_compact_order(&list, ts);
        // получатель 2: наоборот
        recv_ba.apply_compact_order(&list, ts);
        recv_ba.apply_move_task(&moved, key, ts);

        for r in [&recv_ab, &recv_ba] {
            assert_eq!(ids(r), ids(&sender));
            assert_eq!(keys(r), keys(&sender));
        }
        assert_eq!(ids(&sender), ["a", "d", "b", "c"]);
        assert_eq!(keys(&sender), [0.0, 1000.0, 2000.0, 3000.0]);

        // Запоздавший MoveTask из старого пространства (ts раньше компакции).
        assert!(!sender.apply_move_task("c", 1e-9, ts - 10));
        assert_eq!(keys(&sender), [0.0, 1000.0, 2000.0, 3000.0]);
    }

    #[test]
    fn compact_order_op_roundtrips_through_json() {
        use crate::sync::oplog::OpKind;
        let op = OpKind::CompactOrder { project_id: "p".into(), order: order(&["a", "b"]) };
        let line = serde_json::to_string(&op).unwrap();
        assert!(line.contains("COMPACT_ORDER"), "{line}");
        match serde_json::from_str::<OpKind>(&line).unwrap() {
            OpKind::CompactOrder { project_id, order: o } => {
                assert_eq!(project_id, "p");
                assert_eq!(o, order(&["a", "b"]));
            }
            _ => panic!("не тот вариант"),
        }
        assert_eq!(op.project_id(), Some("p"));
        assert_eq!(op.task_id(), None);
    }

    #[test]
    fn move_task_already_applied_then_compaction_on_stale_receiver() {
        // у получателя нет задачи "c", которая есть у отправителя —
        // ранги остальных всё равно совпадают с отправителем.
        let mut sender = proj(&[("a", 0.0, 1), ("b", 1.0, 1), ("c", 2.0, 1), ("d", 3.0, 1)]);
        let mut recv   = proj(&[("a", 0.0, 1), ("b", 1.0, 1), ("d", 3.0, 1)]);
        let list = sender.compact_order(10);
        recv.apply_compact_order(&list, 10);
        assert_eq!(recv.subs["d"].order_key, sender.subs["d"].order_key);
        assert_eq!(recv.subs["b"].order_key, sender.subs["b"].order_key);
    }
}
