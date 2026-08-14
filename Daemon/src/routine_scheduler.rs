use crate::project::Routine;

#[cfg(windows)]
#[allow(non_snake_case, non_camel_case_types)]
mod win_time {
    #[repr(C)]
    pub struct SYSTEMTIME {
        pub wYear: u16, pub wMonth: u16, pub wDayOfWeek: u16, pub wDay: u16,
        pub wHour: u16, pub wMinute: u16, pub wSecond: u16, pub wMilliseconds: u16,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        pub fn GetLocalTime(lpSystemTime: *mut SYSTEMTIME);
        pub fn GetSystemTime(lpSystemTime: *mut SYSTEMTIME);
    }
}

#[cfg(windows)]
fn local_offset_secs() -> i64 {
    use win_time::{GetLocalTime, GetSystemTime, SYSTEMTIME};
    unsafe {
        let mut lt: SYSTEMTIME = std::mem::zeroed();
        let mut st: SYSTEMTIME = std::mem::zeroed();
        GetLocalTime(&mut lt);
        GetSystemTime(&mut st);
        let l = days_from_civil(lt.wYear as i32, lt.wMonth as u32, lt.wDay as u32) * 86400
            + lt.wHour as i64 * 3600 + lt.wMinute as i64 * 60 + lt.wSecond as i64;
        let u = days_from_civil(st.wYear as i32, st.wMonth as u32, st.wDay as u32) * 86400
            + st.wHour as i64 * 3600 + st.wMinute as i64 * 60 + st.wSecond as i64;
        l - u
    }
}

#[cfg(not(windows))]
fn local_offset_secs() -> i64 { 0 }

fn current_time() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn local_now() -> u64 {
    (current_time() as i64 + local_offset_secs()).max(0) as u64
}

pub(crate) fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m, d)
}

fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = (if m <= 2 { y - 1 } else { y }) as i64;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as i64;
    let mp = ((m as i64 + 9) % 12) as i64;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn days_in_month(month: u32, year: i32) -> u32 {
    const DAYS: [u32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    if month == 2 && (year % 4 == 0 && year % 100 != 0 || year % 400 == 0) { 29 }
    else { DAYS[(month - 1) as usize] }
}

fn weekday_of(days: i64) -> i64 {
    (days + 3).rem_euclid(7)
}

fn weekday_code(s: &str) -> Option<i64> {
    match s {
        "mo" => Some(0), "tu" => Some(1), "we" => Some(2), "th" => Some(3),
        "fr" => Some(4), "sa" => Some(5), "su" => Some(6),
        _ => None,
    }
}

fn parse_hm(s: &str) -> Option<(u32, u32)> {
    let (h, m) = s.split_once(':')?;
    Some((h.parse().ok()?, m.parse().ok()?))
}

pub fn last_week_occurrence(entries: &[String], now: u64) -> Option<u64> {
    let now = now as i64;
    let now_days = now.div_euclid(86400);
    let now_wd = weekday_of(now_days);

    entries.iter().filter_map(|e| {
        let (wd_s, hm_s) = e.split_once(' ')?;
        let wd = weekday_code(wd_s)?;
        let (h, m) = parse_hm(hm_s)?;
        let back = (now_wd - wd).rem_euclid(7);
        let mut candidate = (now_days - back) * 86400 + h as i64 * 3600 + m as i64 * 60;
        if candidate > now { candidate -= 7 * 86400; }
        Some(candidate)
    }).max().map(|v| v.max(0) as u64)
}

pub fn last_month_occurrence(entries: &[String], now: u64) -> Option<u64> {
    let now_i = now as i64;
    let now_days = now_i.div_euclid(86400);
    let (year, month, _) = civil_from_days(now_days);

    entries.iter().filter_map(|e| {
        let (day_s, hm_s) = e.split_once(' ')?;
        let day: u32 = day_s.parse().ok()?;
        let (h, m) = parse_hm(hm_s)?;

        let mut y = year;
        let mut mo = month;
        for _ in 0..12 {
            if day <= days_in_month(mo, y) {
                let candidate = days_from_civil(y, mo, day) * 86400 + h as i64 * 3600 + m as i64 * 60;
                if candidate <= now_i {
                    return Some(candidate);
                }
            }
            if mo == 1 { mo = 12; y -= 1; } else { mo -= 1; }
        }
        None
    }).max().map(|v| v.max(0) as u64)
}

pub fn last_direct_occurrence(entries: &[String], now: u64) -> Option<u64> {
    let now_i = now as i64;
    entries.iter().filter_map(|e| direct_entry_secs(e))
        .filter(|&t| t <= now_i)
        .max()
        .map(|v| v.max(0) as u64)
}

fn direct_entry_secs(entry: &str) -> Option<i64> {
    let (date_s, hm_s) = entry.split_once(' ')?;
    let mut it = date_s.splitn(3, '-');
    let y: i32 = it.next()?.parse().ok()?;
    let m: u32 = it.next()?.parse().ok()?;
    let d: u32 = it.next()?.parse().ok()?;
    let (h, mi) = parse_hm(hm_s)?;
    Some(days_from_civil(y, m, d) * 86400 + h as i64 * 3600 + mi as i64 * 60)
}

pub const NOTIFY_WINDOW_SECS: u64 = 60;

pub fn due_occurrence(routine: &Routine, now: u64) -> Option<u64> {
    let candidates = [
        routine.week.as_deref().and_then(|e| last_week_occurrence(e, now)),
        routine.month.as_deref().and_then(|e| last_month_occurrence(e, now)),
        routine.direct.as_deref().and_then(|e| last_direct_occurrence(e, now)),
    ];
    candidates.into_iter().flatten().max().filter(|&t| t > routine.last_triggered_at)
}

pub fn is_due(routine: &Routine, now: u64) -> bool {
    due_occurrence(routine, now).is_some()
}

pub fn prune_expired_direct(routine: &mut Routine, now: u64) {
    let Some(entries) = routine.direct.take() else { return; };
    let now_i = now as i64;
    let kept: Vec<String> = entries.into_iter()
        .filter(|e| direct_entry_secs(e).map_or(true, |t| t >= now_i))
        .collect();
    if !kept.is_empty() {
        routine.direct = Some(kept);
    }
}
