use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone)]
pub struct PeerEntry {
    pub device_id:   String,
    pub device_name: String,
    pub token:       String,
    /// Last known LAN IP — populated by discovery (Stage 4) or set manually.
    pub ip_hint:     Option<String>,
    /// Порт HTTP-сервера ЭТОГО пира (у каждого устройства свой, настраивается
    /// отдельно). Узнаётся из тел request_sync/accept_sync, из discovery и
    /// обновляется из параметра `port=` аутентифицированных запросов пира.
    /// У записей, сохранённых до появления поля, — дефолтный порт.
    #[serde(default = "super::server::default_port")]
    pub port:        u16,
    /// Unix timestamp (seconds) последнего ОСМЫСЛЕННОГО ответа пира на наш
    /// опрос: успешный /1/ops, 403 (отвязали) или /hello с несовместимой
    /// версией. Пока пир на связи, обновляется каждый цикл; когда он уходит
    /// в оффлайн, значение замирает — так одно поле честно годится и для
    /// "Онлайн (...)", и для "Оффлайн (...)". Не меняется, если пир не
    /// ответил совсем или по этому адресу ответило другое устройство.
    #[serde(default)]
    pub last_synced_at: Option<u64>,
    /// Пир ответил 403 на наш токен — нас отвязали. Последнее ИЗВЕСТНОЕ
    /// состояние: переживает перезапуск, чтобы вкладка не показывала
    /// "Оффлайн" до первого опроса. Сбрасывается ответом 200 на /1/ops
    /// (или новым пейрингом — add() заменяет запись целиком); при отсутствии
    /// ответа не трогается.
    #[serde(default)]
    pub revoked: bool,
    /// /hello пира ответил, но нашей версии протокола у него нет — /ops
    /// мы ему не зовём. Тоже последнее известное состояние (см. `revoked`).
    #[serde(default)]
    pub incompatible: bool,
    /// Обновляется на каждом успешном пуле через /1/hello (см. engine.rs) —
    /// пир может сменить платформу/переустановиться, так что не считаем
    /// зафиксированным раз и навсегда, как token.
    #[serde(default)]
    pub device_type: super::DeviceType,
}

pub struct Peers {
    list: Vec<PeerEntry>,
    dir:  PathBuf,
}

impl Peers {
    pub fn load(dir: &Path) -> Self {
        let list = std::fs::read_to_string(dir.join("trusted_peers.json")).ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self { list, dir: dir.to_owned() }
    }

    /// Для тестов: Peers поверх готового списка и произвольной папки.
    #[cfg(test)]
    pub fn for_test(list: Vec<PeerEntry>, dir: &Path) -> Self {
        Self { list, dir: dir.to_owned() }
    }

    pub fn save(&self) {
        if let Ok(j) = serde_json::to_string_pretty(&self.list) {
            let _ = std::fs::write(self.dir.join("trusted_peers.json"), j);
        }
    }

    pub fn all(&self) -> &[PeerEntry] { &self.list }

    pub fn list_mut(&mut self) -> impl Iterator<Item = &mut PeerEntry> {
        self.list.iter_mut()
    }

    pub fn find_by_token(&self, token: &str) -> Option<&PeerEntry> {
        self.list.iter().find(|p| p.token == token)
    }

    pub fn find_by_id(&self, device_id: &str) -> Option<&PeerEntry> {
        self.list.iter().find(|p| p.device_id == device_id)
    }

    /// Upsert: replaces existing entry for same device_id, else appends.
    pub fn add(&mut self, entry: PeerEntry) {
        match self.list.iter_mut().find(|p| p.device_id == entry.device_id) {
            Some(existing) => *existing = entry,
            None           => self.list.push(entry),
        }
        self.save();
    }

    pub fn remove(&mut self, device_id: &str) {
        self.list.retain(|p| p.device_id != device_id);
        self.save();
    }

    /// Обновить адрес пира (IP + порт его сервера). Сохраняет на диск только
    /// при реальном изменении. Возвращает true, если адрес изменился.
    pub fn update_addr(&mut self, device_id: &str, ip: &str, port: u16) -> bool {
        let Some(p) = self.list.iter_mut().find(|p| p.device_id == device_id) else { return false; };
        if p.ip_hint.as_deref() == Some(ip) && p.port == port { return false; }
        p.ip_hint = Some(ip.to_owned());
        p.port    = port;
        self.save();
        true
    }

    /// Записать результат опроса пира в ПАМЯТЬ (на диск не пишет — движок
    /// вызывает save() один раз за цикл, если хоть кто-то ответил). `None`
    /// в поле = "не менять". Возвращает true, если что-то изменилось.
    pub fn apply_poll(&mut self, device_id: &str, u: PollUpdate) -> bool {
        let Some(p) = self.list.iter_mut().find(|p| p.device_id == device_id) else { return false; };
        let mut changed = false;
        if let Some(ts) = u.contact_ts {
            if p.last_synced_at != Some(ts) { p.last_synced_at = Some(ts); changed = true; }
        }
        if let Some(r) = u.revoked {
            if p.revoked != r { p.revoked = r; changed = true; }
        }
        if let Some(i) = u.incompatible {
            if p.incompatible != i { p.incompatible = i; changed = true; }
        }
        changed
    }
}

/// Что именно записать в запись пира после опроса (см. Peers::apply_poll).
#[derive(Clone, Copy, Default)]
pub struct PollUpdate {
    pub contact_ts:   Option<u64>,
    pub revoked:      Option<bool>,
    pub incompatible: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str) -> PeerEntry {
        PeerEntry {
            device_id: id.into(), device_name: "n".into(), token: "t".into(),
            ip_hint: Some("127.0.0.1".into()), port: 1, last_synced_at: None,
            revoked: false, incompatible: false, device_type: Default::default(),
        }
    }
    fn peers(list: Vec<PeerEntry>) -> Peers { Peers { list, dir: PathBuf::from(".") } }

    #[test]
    fn old_file_without_new_fields_loads_with_defaults() {
        let j = r#"[{"device_id":"a","device_name":"A","token":"t","ip_hint":null,
                     "port":5,"last_synced_at":42,"device_type":"desktop"}]"#;
        let v: Vec<PeerEntry> = serde_json::from_str(j).unwrap();
        assert_eq!(v[0].last_synced_at, Some(42));
        assert!(!v[0].revoked && !v[0].incompatible);
    }

    #[test]
    fn apply_poll_none_means_keep_and_reports_change() {
        let mut p = peers(vec![entry("a")]);
        // 403: время + revoked, incompatible сбрасывается
        assert!(p.apply_poll("a", PollUpdate { contact_ts: Some(10), revoked: Some(true), incompatible: Some(false) }));
        let e = p.find_by_id("a").unwrap();
        assert_eq!((e.last_synced_at, e.revoked, e.incompatible), (Some(10), true, false));
        // пустое обновление ничего не меняет и не сообщает об изменении
        assert!(!p.apply_poll("a", PollUpdate::default()));
        let e = p.find_by_id("a").unwrap();
        assert_eq!((e.last_synced_at, e.revoked), (Some(10), true));
        // тот же результат повторно — не "изменение"
        assert!(!p.apply_poll("a", PollUpdate { contact_ts: Some(10), revoked: Some(true), incompatible: Some(false) }));
        // 200: revoked снимается
        assert!(p.apply_poll("a", PollUpdate { contact_ts: Some(20), revoked: Some(false), incompatible: Some(false) }));
        assert!(!p.find_by_id("a").unwrap().revoked);
        // неизвестный пир
        assert!(!p.apply_poll("zzz", PollUpdate { contact_ts: Some(1), ..Default::default() }));
    }

    #[test]
    fn pairing_again_resets_flags() {
        let mut p = peers(vec![entry("a")]);
        p.apply_poll("a", PollUpdate { contact_ts: Some(5), revoked: Some(true), incompatible: Some(false) });
        let fresh = entry("a");
        // add() сохраняет на диск ("."), в тесте это безвредно: подменяем dir
        p.dir = std::env::temp_dir().join("cue_peers_test_dir_unused");
        p.add(fresh);
        let e = p.find_by_id("a").unwrap();
        assert!(!e.revoked && !e.incompatible && e.last_synced_at.is_none());
    }
}