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

pub fn gen_id() -> String {
    static CTR: AtomicU64 = AtomicU64::new(0);
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    let mut n = t
        ^ ((std::process::id() as u64) << 32)
        ^ CTR.fetch_add(1, Ordering::Relaxed).wrapping_mul(0x9e3779b97f4a7c15);
    const CH: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut id = String::with_capacity(12);
    for _ in 0..12 {
        id.push(CH[(n as usize) % 62] as char);
        n = n.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    }
    id
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
    }

    /// true, если у рутины есть ТОЛЬКО direct-записи (нет week/month).
    /// Используется, чтобы понять, может ли "исчерпание" direct-дат
    /// привести к удалению задачи целиком (раздел 2.4 плана).
    pub fn is_direct_only(&self) -> bool {
        self.week.is_none() && self.month.is_none() && self.direct.is_some()
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
            if let Some(occ) = crate::routine_scheduler::due_occurrence(r, now) {
                r.active = true;
                r.last_triggered_at = now;
                crate::routine_scheduler::prune_expired_direct(r, now);
                if now.saturating_sub(occ) <= crate::routine_scheduler::NOTIFY_WINDOW_SECS {
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
            loaded:     true,
            main_edited_at: 0, name_edited_at: 0, color_edited_at: 0,
            order_key: 0.0, order_key_edited_at: 0,
        }
    }

    pub fn main_text(&self) -> Option<&str> {
        self.main.values().next().map(|t| t.text.as_str())
    }

    pub fn save(&self) {
        let file = ProjectFile {
            ver:   1,
            name:  self.name.clone(),
            color: self.color_hex.clone(),
            tasks: TasksFile {
                main: self.main.clone(),
                subs: self.subs.clone(),
            },
            last_edited: current_time(),
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
            task_count:         self.main.len() + self.subs.len(),
            has_active_routine: self.has_active_routine(),
            order_key:          self.order_key,
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

    /// `now` — момент выполнения: для локального клика передаётся
    /// routine_scheduler::local_now(), для входящего по сети CompleteTask-
    /// опа — op.ts самого опа.
    /// Единая точка завершения задачи — не важно, откуда пришёл вызов:
    /// клик по main-слоту или крестик по активной рутине в subs. Ищет
    /// `task_id` сначала в main, потом в subs, применяет поведение по месту
    /// находки. Обычная задача — удаляется. Задача с рутиной — гасится
    /// (active=false) либо удаляется целиком, если рутина исчерпана.
    ///
    /// Важно: если задача найдена уже в subs — НЕ трогаем order_key и
    /// физическую позицию (мутация на месте), физический порядок важен
    /// для отображения в "плоском" режиме. Из main же выход в subs — это
    /// всегда новая запись (позиции раньше не было, терять нечего).
    pub fn complete_task(&mut self, task_id: &str, now: u64, main_ts: u64) -> bool {
        if self.main.contains_key(task_id) {
            let (id, mut task) = self.main.shift_remove_entry(task_id).unwrap();
            if let Some(routine) = task.routine.as_mut() {
                crate::routine_scheduler::prune_expired_direct(routine, now);
                let exhausted = routine.week.is_none()
                    && routine.month.is_none()
                    && routine.direct.as_ref().map_or(true, |d| d.is_empty());
                if !exhausted {
                    routine.active = false;
                    task.order_key = self.next_end_key();
                    self.subs.insert(id, task); // физический конец; display-порядок группирует отдельно
                }
                // exhausted — задача просто никуда не возвращается
            }
            // task.routine было None — обычная задача, просто пропадает

            // Продвигаем в main первую ЭФФЕКТИВНО АКТИВНУЮ sub-задачу (группа
            // active всегда идёт перед not-active благодаря компаратору
            // сортировки в load_all_projects/insert — см. is_active_task).
            // position() — защитная подстраховка на случай рассинхрона
            // сортировки.
            if let Some(pos) = self.subs.iter().position(|(_, t)| is_active_task(t)) {
                let (next_id, next_task) = self.subs.shift_remove_index(pos).unwrap();
                self.main.insert(next_id, next_task);
                self.main_edited_at = main_ts; // UTC, не local_now — тот же базис, что у Promote/AddTask
            }
            return true;
        }

        if self.subs.contains_key(task_id) {
            let has_routine = self.subs.get(task_id).is_some_and(|t| t.routine.is_some());
            if !has_routine {
                self.subs.shift_remove(task_id); // обычная задача в subs — просто удаляется
                return true;
            }
            let routine = self.subs.get_mut(task_id).unwrap().routine.as_mut().unwrap();
            crate::routine_scheduler::prune_expired_direct(routine, now);
            let exhausted = routine.week.is_none()
                && routine.month.is_none()
                && routine.direct.as_ref().map_or(true, |d| d.is_empty());
            if exhausted {
                // Тумбстоун не нужен: результат детерминирован из уже
                // синканных entries + ts, каждое устройство придёт к тому
                // же выводу само.
                self.subs.shift_remove(task_id);
            } else {
                // НЕ трогаем order_key и физическую позицию — задача
                // остаётся ровно там же, просто гаснет.
                self.subs.get_mut(task_id).unwrap().routine.as_mut().unwrap().active = false;
            }
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
    Some(LoadedProject {
        id: id.to_owned(),
        name:       file.name,
        color,
        color_hex:  file.color,
        main:       file.tasks.main,
        subs:       file.tasks.subs,
        created_at: file.created_at,
        loaded:     true,
        main_edited_at: file.main_edited_at, name_edited_at: file.name_edited_at, color_edited_at: file.color_edited_at,
        order_key: file.order_key, order_key_edited_at: file.order_key_edited_at,
    })
}

/// Синхронно загружает активный проект на старте, с фолбэком для
/// битого/отсутствующего файла — см. Cue_Мёрж_Батча_И_Битые_Файлы.txt,
/// "СЦЕНАРИЙ: АКТИВНЫЙ ПРОЕКТ БИТ/НЕДОСТУПЕН ИМЕННО НА СТАРТЕ".
///
/// ГОТОВО, НО ПОКА НИКЕМ НЕ ВЫЗЫВАЕТСЯ. Сегодня (полная синхронная
/// загрузка, Этап 3) этот сценарий уже безобидно покрывается сам —
/// load_all_projects() молча роняет битый файл, а резолвинг активного
/// индекса в main.rs откатывается на index 0 (реальный, уже загруженный
/// проект). Эта функция понадобится на Этапе 5 — в Ветке Б остальные
/// проекты будут только заглушками (tasks: None), переключиться на
/// "другой проект" при ошибке будет физически некуда, показывать нечего.
///
/// manifest.is_empty() решает между двумя ветками:
///   - манифест НЕ пуст (другие проекты есть, но это заглушки) → если
///     last_project_id не смог прочитаться — сразу дефолтный, БЕЗ
///     каскада по другим id манифеста (риск: при системной порче диска
///     цепочка попыток может затянуть старт на неопределённое время).
///     Если last_project_id вообще не задан (None, не порча данных, а
///     его отсутствие) — ОДНА попытка (без цепочки) на первый id из
///     манифеста, и только если она тоже не удалась — дефолтный.
///   - манифест пуст → тривиальный случай, просто пробуем
///     last_project_id, иначе сразу дефолтный.
pub fn load_active_with_fallback(
    manifest: &crate::manifest::Manifest,
    last_project_id: Option<&str>,
) -> LoadedProject {
    if !manifest.is_empty() {
        if let Some(id) = last_project_id {
            return load_one(id).unwrap_or_else(create_default_project);
        }
        if let Some(first_id) = manifest.keys().next() {
            if let Some(p) = load_one(first_id) {
                return p;
            }
        }
        return create_default_project();
    }

    last_project_id
        .and_then(load_one)
        .unwrap_or_else(create_default_project)
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
            let proj = LoadedProject {
                id,
                name:       file.name,
                color,
                color_hex:  file.color,
                main:       file.tasks.main,
                subs:       file.tasks.subs,
                created_at: file.created_at,
                loaded:     true,
                main_edited_at: file.main_edited_at, name_edited_at: file.name_edited_at, color_edited_at: file.color_edited_at,
                order_key: file.order_key, order_key_edited_at: file.order_key_edited_at,
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
    let proj = LoadedProject::new(
        gen_id(),
        "Cue".to_owned(),
        Color32::from_rgb(74, 144, 217),
        current_time(),
    );
    proj.save();
    proj
}