//! Live room state machine.
use std::collections::{HashMap, HashSet};
#[derive(Default)]
pub struct Live {
    rooms: HashMap<String, HashSet<String>>,
    capacity: usize,
    seats: HashMap<String, Vec<Option<String>>>,
    muted: HashSet<(String, String)>,
    robots: HashMap<String, u32>,
}
impl Live {
    pub fn new(capacity: usize) -> Self { Self { rooms: HashMap::new(), capacity: capacity.max(1), seats: HashMap::new(), muted: HashSet::new(), robots: HashMap::new() } }
    pub fn join(&mut self, room: &str, user: &str) -> bool {
        let r = self.rooms.entry(room.to_string()).or_default();
        if r.len() >= self.capacity && !r.contains(user) { return false; }
        r.insert(user.to_string())
    }
    pub fn leave(&mut self, room: &str, user: &str) { if let Some(r)=self.rooms.get_mut(room) { r.remove(user); } }
    pub fn members(&self, room: &str) -> usize { self.rooms.get(room).map(|r| r.len()).unwrap_or(0) }
    pub fn rooms(&self) -> usize { self.rooms.len() }

    pub fn seat_count(&self, room: &str) -> usize {
        self.seats.get(room).map(|s| s.len()).unwrap_or(8)
    }

    pub fn seat_list(&self, room: &str) -> Vec<Option<String>> {
        self.seats
            .get(room)
            .cloned()
            .unwrap_or_else(|| vec![None; 8])
    }

    pub fn take_seat(&mut self, room: &str, user: &str, index: usize) -> bool {
        let seats = self.seats.entry(room.to_string()).or_insert_with(|| vec![None; 8]);
        if index >= seats.len() {
            return false;
        }
        if seats[index].as_deref() == Some(user) {
            return true;
        }
        if seats[index].is_some() {
            return false;
        }
        for slot in seats.iter_mut() {
            if slot.as_deref() == Some(user) {
                *slot = None;
            }
        }
        seats[index] = Some(user.to_string());
        self.rooms.entry(room.to_string()).or_default().insert(user.to_string());
        true
    }

    pub fn leave_seat(&mut self, room: &str, user: &str) -> bool {
        let Some(seats) = self.seats.get_mut(room) else { return false };
        let mut found = false;
        for slot in seats.iter_mut() {
            if slot.as_deref() == Some(user) {
                *slot = None;
                found = true;
            }
        }
        found
    }

    pub fn kick(&mut self, room: &str, user: &str) -> bool {
        let removed = self
            .rooms
            .get_mut(room)
            .map(|r| r.remove(user))
            .unwrap_or(false);
        if let Some(seats) = self.seats.get_mut(room) {
            for slot in seats.iter_mut() {
                if slot.as_deref() == Some(user) {
                    *slot = None;
                }
            }
        }
        self.muted.remove(&(room.to_string(), user.to_string()));
        removed
    }

    pub fn set_muted(&mut self, room: &str, user: &str, muted: bool) -> bool {
        if muted {
            self.muted.insert((room.to_string(), user.to_string()));
        } else {
            self.muted.remove(&(room.to_string(), user.to_string()));
        }
        true
    }

    pub fn muted_users(&self, room: &str) -> Vec<String> {
        self.muted.iter().filter(|(r, _)| r == room).map(|(_, u)| u.clone()).collect()
    }

    pub fn is_muted(&self, room: &str, user: &str) -> bool {
        self.muted.contains(&(room.to_string(), user.to_string()))
    }

    pub fn add_robots(&mut self, room: &str, count: u32) -> u32 {
        let n = self.robots.entry(room.to_string()).or_default();
        *n = n.saturating_add(count);
        *n
    }

    pub fn robots(&self, room: &str) -> u32 {
        self.robots.get(room).copied().unwrap_or(0)
    }

    pub fn to_json(&self) -> String {
        let rooms: HashMap<String, Vec<String>> = self
            .rooms
            .iter()
            .map(|(k, v)| (k.clone(), v.iter().cloned().collect()))
            .collect();
        serde_json::json!({
            "rooms": rooms,
            "capacity": self.capacity,
            "seats": self.seats,
            "muted": self.muted.iter().cloned().collect::<Vec<_>>(),
            "robots": self.robots,
        })
        .to_string()
    }

    pub fn from_json(s: &str) -> Self {
        let value: serde_json::Value = serde_json::from_str(s).unwrap_or_default();
        let capacity = value["capacity"].as_u64().unwrap_or(1000) as usize;
        let mut live = Self::new(capacity);
        if let Some(map) = value["rooms"].as_object() {
            for (room, members) in map {
                if let Some(arr) = members.as_array() {
                    let set = live.rooms.entry(room.clone()).or_default();
                    for m in arr {
                        if let Some(u) = m.as_str() { set.insert(u.to_string()); }
                    }
                }
            }
        }
        live
    }

    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() { std::fs::create_dir_all(dir)?; }
        std::fs::write(path, self.to_json())
    }

    pub fn load(path: &std::path::Path) -> Self {
        std::fs::read_to_string(path).map(|s| Self::from_json(&s)).unwrap_or_else(|_| Self::new(1000))
    }
}
#[cfg(test)]
mod seat_tests {
    use super::*;
    #[test]
    fn seats_mute_kick() {
        let mut l = Live::new(10);
        assert!(l.take_seat("r", "a", 0));
        assert!(!l.take_seat("r", "b", 0));
        assert!(l.take_seat("r", "b", 1));
        assert!(l.take_seat("r", "a", 2));
        assert_eq!(l.seat_list("r")[0], None);
        assert_eq!(l.seat_list("r")[2].as_deref(), Some("a"));
        assert!(l.leave_seat("r", "a"));
        assert!(l.set_muted("r", "b", true));
        assert!(l.is_muted("r", "b"));
        assert!(l.kick("r", "b"));
        assert!(!l.is_muted("r", "b"));
        assert_eq!(l.add_robots("r", 3), 3);
        assert_eq!(l.robots("r"), 3);
    }
}

#[cfg(test)] mod tests { use super::*; #[test] fn cap() { let mut l=Live::new(1); assert!(l.join("r","a")); assert!(!l.join("r","b")); l.leave("r","a"); assert!(l.join("r","b")); } }

#[derive(Default, serde::Serialize, serde::Deserialize)]
pub struct Gifts {
    totals: HashMap<String, u64>,
    log: HashMap<String, Vec<(String, u32, u32)>>,
    #[serde(default)]
    events: Vec<GiftEvent>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GiftEvent {
    pub ts: u64,
    pub room: String,
    pub user: String,
    pub gift: u32,
    pub count: u32,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl Gifts {
    pub fn send(&mut self, room: &str, user: &str, gift: u32, count: u32) -> u64 {
        let n = count.max(1) as u64;
        *self.totals.entry(room.to_string()).or_default() += n;
        self.log
            .entry(room.to_string())
            .or_default()
            .push((user.to_string(), gift, n as u32));
        self.events.push(GiftEvent {
            ts: now_ms(),
            room: room.to_string(),
            user: user.to_string(),
            gift,
            count: n as u32,
        });
        self.totals[room]
    }

    pub fn total(&self, room: &str) -> u64 {
        self.totals.get(room).copied().unwrap_or(0)
    }

    pub fn ranking(&self, room: &str) -> Vec<(String, u64)> {
        let mut map: HashMap<String, u64> = HashMap::new();
        for (user, _, count) in self.log.get(room).into_iter().flatten() {
            *map.entry(user.clone()).or_default() += *count as u64;
        }
        let mut out: Vec<(String, u64)> = map.into_iter().collect();
        out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        out
    }

    pub fn ranking_value(&self, room: &str) -> Vec<(String, u64)> {
        let mut map: HashMap<String, u64> = HashMap::new();
        for (user, gift, count) in self.log.get(room).into_iter().flatten() {
            *map.entry(user.clone()).or_default() += (*gift as u64) * (*count as u64);
        }
        let mut out: Vec<(String, u64)> = map.into_iter().collect();
        out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        out
    }

    pub fn recent_events(&self, room: &str, limit: usize) -> Vec<GiftEvent> {
        self.events_since(room, 0, limit)
    }

    pub fn events_since(&self, room: &str, since: u64, limit: usize) -> Vec<GiftEvent> {
        let mut out: Vec<GiftEvent> = self
            .events
            .iter()
            .filter(|e| e.room == room && e.ts >= since)
            .rev()
            .take(limit)
            .cloned()
            .collect();
        out.reverse();
        out
    }

    pub fn ranking_since(&self, room: &str, since: u64) -> Vec<(String, u64)> {
        let mut map: HashMap<String, u64> = HashMap::new();
        for e in self.events.iter().filter(|e| e.room == room && e.ts >= since) {
            *map.entry(e.user.clone()).or_default() += e.count as u64;
        }
        let mut out: Vec<(String, u64)> = map.into_iter().collect();
        out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        out
    }

    pub fn last(&self, room: &str) -> Option<&(String, u32, u32)> {
        self.log.get(room).and_then(|v| v.last())
    }
}

#[derive(Default)]
pub struct Pk {
    scores: HashMap<String, (u32, u32)>,
}

impl Pk {
    pub fn add(&mut self, room: &str, side: u8, points: u32) {
        let e = self.scores.entry(room.to_string()).or_default();
        if side == 0 { e.0 += points } else { e.1 += points }
    }

    pub fn score(&self, room: &str) -> (u32, u32) {
        self.scores.get(room).copied().unwrap_or((0, 0))
    }
}

#[cfg(test)]
mod gift_tests {
    use super::*;
    #[test]
    fn gift_and_pk() {
        let mut g = Gifts::default();
        assert_eq!(g.send("r", "a", 1, 3), 3);
        assert_eq!(g.total("r"), 3);
        assert_eq!(g.last("r").unwrap().2, 3);
        g.send("r", "b", 1, 5);
        let rank = g.ranking("r");
        assert_eq!(rank[0], ("b".to_string(), 5));
        assert_eq!(rank[1], ("a".to_string(), 3));
        g.send("r", "c", 100, 1);
        let value = g.ranking_value("r");
        assert_eq!(value[0], ("c".to_string(), 100));
        assert_eq!(g.ranking_since("r", 0)[0], ("b".to_string(), 5));
        assert!(g.recent_events("r", 10).len() >= 3);
        let mut p = Pk::default();
        p.add("r", 0, 10);
        p.add("r", 1, 7);
        assert_eq!(p.score("r"), (10, 7));
    }
}

static ROOMS: std::sync::OnceLock<std::sync::Mutex<Live>> = std::sync::OnceLock::new();
static GIFTS: std::sync::OnceLock<std::sync::Mutex<Gifts>> = std::sync::OnceLock::new();
static PK: std::sync::OnceLock<std::sync::Mutex<Pk>> = std::sync::OnceLock::new();

fn rooms() -> &'static std::sync::Mutex<Live> {
    ROOMS.get_or_init(|| std::sync::Mutex::new(Live::new(1000)))
}
fn gift_path() -> std::path::PathBuf {
    std::env::var("M7S_GIFT_FILE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("data/gifts.json"))
}

fn gifts() -> &'static std::sync::Mutex<Gifts> {
    GIFTS.get_or_init(|| {
        let loaded = std::fs::read_to_string(gift_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        std::sync::Mutex::new(loaded)
    })
}

fn save_gifts(g: &Gifts) {
    let path = gift_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(data) = serde_json::to_vec(g) {
        let _ = std::fs::write(path, data);
    }
}
fn pk() -> &'static std::sync::Mutex<Pk> {
    PK.get_or_init(Default::default)
}

pub fn join_room(room: &str, user: &str) -> bool {
    rooms().lock().unwrap_or_else(|e| e.into_inner()).join(room, user)
}
pub fn leave_room(room: &str, user: &str) {
    rooms().lock().unwrap_or_else(|e| e.into_inner()).leave(room, user);
}
pub fn room_names() -> Vec<String> {
    rooms().lock().unwrap_or_else(|e| e.into_inner()).rooms.iter().filter(|(_, v)| !v.is_empty()).map(|(k, _)| k.clone()).collect()
}

pub fn take_seat(room: &str, user: &str, index: usize) -> bool {
    rooms().lock().unwrap_or_else(|e| e.into_inner()).take_seat(room, user, index)
}
pub fn leave_seat(room: &str, user: &str) -> bool {
    rooms().lock().unwrap_or_else(|e| e.into_inner()).leave_seat(room, user)
}
pub fn seats(room: &str) -> Vec<Option<String>> {
    rooms().lock().unwrap_or_else(|e| e.into_inner()).seat_list(room)
}
pub fn kick(room: &str, user: &str) -> bool {
    rooms().lock().unwrap_or_else(|e| e.into_inner()).kick(room, user)
}
pub fn set_muted(room: &str, user: &str, muted: bool) -> bool {
    rooms().lock().unwrap_or_else(|e| e.into_inner()).set_muted(room, user, muted)
}
pub fn muted_users(room: &str) -> Vec<String> {
    rooms().lock().unwrap_or_else(|e| e.into_inner()).muted_users(room)
}
pub fn is_muted(room: &str, user: &str) -> bool {
    rooms().lock().unwrap_or_else(|e| e.into_inner()).is_muted(room, user)
}
pub fn add_robots(room: &str, count: u32) -> u32 {
    rooms().lock().unwrap_or_else(|e| e.into_inner()).add_robots(room, count)
}
pub fn robots(room: &str) -> u32 {
    rooms().lock().unwrap_or_else(|e| e.into_inner()).robots(room)
}

pub fn members(room: &str) -> usize {
    rooms().lock().unwrap_or_else(|e| e.into_inner()).members(room)
}
pub fn send_gift(room: &str, user: &str, gift: u32, count: u32) -> u64 {
    let mut guard = gifts().lock().unwrap_or_else(|e| e.into_inner());
    let total = guard.send(room, user, gift, count);
    if let Some(event) = guard.events.last() {
        let path = event_log_path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            use std::io::Write;
            let _ = writeln!(file, "{}", serde_json::to_string(event).unwrap_or_default());
            let max = std::env::var("M7S_GIFT_EVENT_MAX").ok().and_then(|v| v.parse::<usize>().ok()).unwrap_or(1_000_000);
            if max > 0 {
                trim_jsonl(&path, max);
            }
        }
    }
    save_gifts(&guard);
    total
}
pub fn gift_total(room: &str) -> u64 {
    gifts().lock().unwrap_or_else(|e| e.into_inner()).total(room)
}
pub fn pk_add(room: &str, side: u8, points: u32) {
    pk().lock().unwrap_or_else(|e| e.into_inner()).add(room, side, points);
}
pub fn pk_score(room: &str) -> (u32, u32) {
    pk().lock().unwrap_or_else(|e| e.into_inner()).score(room)
}

#[cfg(test)]
mod room_api_tests {
    use super::*;
    #[test]
    fn join_gift_pk() {
        let room = format!("api-{}", std::process::id());
        assert!(join_room(&room, "u1"));
        assert_eq!(members(&room), 1);
        let base = gift_total(&room);
        assert_eq!(send_gift(&room, "u1", 2, 5), base + 5);
        pk_add(&room, 1, 9);
        assert_eq!(pk_score(&room), (0, 9));
        leave_room(&room, "u1");
        assert_eq!(members(&room), 0);
    }
}

#[cfg(test)]
mod persist_tests {
    use super::*;

    #[test]
    fn save_and_load() {
        let path = std::env::temp_dir().join(format!("m7s-rooms-{}.json", std::process::id()));
        let mut live = Live::new(10);
        assert!(live.join("r1", "alice"));
        assert!(live.join("r1", "bob"));
        live.save(&path).unwrap();
        let loaded = Live::load(&path);
        assert_eq!(loaded.members("r1"), 2);
        assert_eq!(loaded.rooms(), 1);
        let json = live.to_json();
        assert!(json.contains("alice"));
        let _ = std::fs::remove_file(path);
    }
}

#[derive(Clone, serde::Serialize)]
pub struct RoomInfo {
    pub name: String,
    pub title: String,
    pub created: u64,
    pub closed: bool,
}

static ROOM_INFOS: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, RoomInfo>>> =
    std::sync::OnceLock::new();

fn room_infos() -> &'static std::sync::Mutex<std::collections::HashMap<String, RoomInfo>> {
    ROOM_INFOS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn create_room(name: &str, title: &str) -> bool {
    let mut infos = room_infos().lock().unwrap_or_else(|e| e.into_inner());
    if infos.contains_key(name) {
        return false;
    }
    infos.insert(
        name.to_string(),
        RoomInfo {
            name: name.to_string(),
            title: title.to_string(),
            created: now_secs(),
            closed: false,
        },
    );
    true
}

pub fn close_room(name: &str) -> bool {
    let mut infos = room_infos().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(r) = infos.get_mut(name) {
        if r.closed {
            return false;
        }
        r.closed = true;
        return true;
    }
    if room_names().iter().any(|r| r == name) {
        infos.insert(
            name.to_string(),
            RoomInfo { name: name.to_string(), title: String::new(), created: now_secs(), closed: true },
        );
        return true;
    }
    false
}
pub fn room_infos_list() -> Vec<RoomInfo> {
    let mut v: Vec<RoomInfo> = room_infos()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .values()
        .cloned()
        .collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;

    #[test]
    fn create_close_room() {
        let name = format!("life-{}", std::process::id());
        assert!(create_room(&name, "Title"));
        assert!(!create_room(&name, "Again"));
        assert!(room_infos_list().iter().any(|r| r.name == name && r.title == "Title" && !r.closed));
        assert!(close_room(&name));
        assert!(!close_room(&name));
        assert!(room_infos_list().iter().any(|r| r.name == name && r.closed));
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ChatMessage {
    pub user: String,
    pub text: String,
    pub ts: u64,
}

fn chats() -> &'static std::sync::Mutex<std::collections::HashMap<String, std::collections::VecDeque<ChatMessage>>> {
    static CHATS: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, std::collections::VecDeque<ChatMessage>>>> = std::sync::OnceLock::new();
    CHATS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

fn banned_words() -> Vec<String> {
    let extra = std::env::var("M7S_LIVE_BANNED_WORDS").unwrap_or_default();
    let mut words = vec!["badword".to_string(), "forbidden".to_string()];
    words.extend(extra.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string));
    words
}

pub fn send_chat(room: &str, user: &str, text: &str) -> Result<ChatMessage, String> {
    if user.is_empty() || text.is_empty() {
        return Err("user/text required".to_string());
    }
    if text.chars().count() > 200 {
        return Err("message too long".to_string());
    }
    if is_muted(room, user) {
        return Err("user muted".to_string());
    }
    let lower = text.to_lowercase();
    if banned_words().iter().any(|w| lower.contains(&w.to_lowercase())) {
        return Err("banned word".to_string());
    }
    let msg = ChatMessage {
        user: user.to_string(),
        text: text.to_string(),
        ts: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    };
    let mut map = chats().lock().unwrap_or_else(|e| e.into_inner());
    let log = map.entry(room.to_string()).or_default();
    log.push_back(msg.clone());
    while log.len() > 200 {
        log.pop_front();
    }
    Ok(msg)
}

pub fn chat_history(room: &str, limit: usize) -> Vec<ChatMessage> {
    let map = chats().lock().unwrap_or_else(|e| e.into_inner());
    let log = map.get(room);
    let Some(log) = log else { return Vec::new() };
    let start = log.len().saturating_sub(limit.max(1));
    log.iter().skip(start).cloned().collect()
}

pub fn clear_chat(room: &str) {
    chats().lock().unwrap_or_else(|e| e.into_inner()).remove(room);
}

#[cfg(test)]
mod chat_tests {
    use super::*;

    #[test]
    fn send_mute_ban_history() {
        let room = format!("chat-{}", std::process::id());
        clear_chat(&room);
        let msg = send_chat(&room, "alice", "hello").unwrap();
        assert_eq!(msg.text, "hello");
        assert_eq!(chat_history(&room, 10).len(), 1);
        assert!(send_chat(&room, "alice", "badword here").is_err());
        set_muted(&room, "alice", true);
        assert!(send_chat(&room, "alice", "hello again").is_err());
        set_muted(&room, "alice", false);
        assert!(send_chat(&room, "alice", "ok").is_ok());
        assert_eq!(chat_history(&room, 1).len(), 1);
        clear_chat(&room);
        assert!(chat_history(&room, 10).is_empty());
    }
}

pub struct BotHandle {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl BotHandle {
    pub fn stop(&self) {
        self.stop.store(true, std::sync::atomic::Ordering::Release);
    }
}

impl Drop for BotHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn start_bot(
    room: &str,
    user: &str,
    messages: Vec<String>,
    interval_ms: u64,
) -> BotHandle {
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = stop.clone();
    let room = room.to_string();
    let user = user.to_string();
    let interval = interval_ms.max(10);
    std::thread::spawn(move || {
        while !flag.load(std::sync::atomic::Ordering::Acquire) {
            for text in &messages {
                if flag.load(std::sync::atomic::Ordering::Acquire) {
                    break;
                }
                apply_bot_action(&room, &user, text);
                std::thread::sleep(std::time::Duration::from_millis(interval));
            }
            if messages.is_empty() {
                break;
            }
        }
    });
    BotHandle { stop }
}

fn bots() -> &'static std::sync::Mutex<std::collections::HashMap<String, BotHandle>> {
    static BOTS: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, BotHandle>>> = std::sync::OnceLock::new();
    BOTS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

pub fn start_bot_room(room: &str, user: &str, messages: Vec<String>, interval_ms: u64) -> bool {
    if room.is_empty() || user.is_empty() || messages.is_empty() {
        return false;
    }
    let handle = start_bot(room, user, messages, interval_ms);
    bots()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(room.to_string(), handle);
    true
}

pub fn stop_bot_room(room: &str) -> bool {
    bots()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(room)
        .is_some()
}

#[cfg(test)]
mod bot_tests {
    use super::*;
    #[test]
    fn bot_posts_and_stops() {
        let room = format!("bot-{}", std::process::id());
        clear_chat(&room);
        assert!(start_bot_room(&room, "bot1", vec!["hello".to_string(), "world".to_string()], 10));
        std::thread::sleep(std::time::Duration::from_millis(120));
        let history = chat_history(&room, 20);
        assert!(history.len() >= 2, "{history:?}");
        assert!(history.iter().all(|m| m.user == "bot1"));
        assert!(stop_bot_room(&room));
        clear_chat(&room);
    }
    #[test]
    fn bot_actions() {
        let room = format!("bota-{}", std::process::id());
        clear_chat(&room);
        let msgs = vec!["join".to_string(), "seat:1".to_string(), "like:5".to_string(), "gift:1,2".to_string(), "hello".to_string()];
        assert!(start_bot_room(&room, "bot2", msgs, 10));
        std::thread::sleep(std::time::Duration::from_millis(150));
        assert_eq!(members(&room), 1);
        assert_eq!(seats(&room)[1].as_deref(), Some("bot2"));
        assert!(robots(&room) >= 5);
        assert!(gift_total(&room) >= 2);
        assert!(chat_history(&room, 10).iter().any(|m| m.text == "hello"));
        assert!(stop_bot_room(&room));
        clear_chat(&room);
    }
}

fn apply_bot_action(room: &str, user: &str, action: &str) {
    if action == "join" {
        join_room(room, user);
        return;
    }
    if action == "leave" {
        leave_room(room, user);
        return;
    }
    if let Some(n) = action.strip_prefix("like:") {
        if let Ok(n) = n.parse::<u32>() {
            add_robots(room, n);
        }
        return;
    }
    if let Some(rest) = action.strip_prefix("seat:") {
        if let Ok(i) = rest.parse::<usize>() {
            take_seat(room, user, i);
        }
        return;
    }
    if let Some(rest) = action.strip_prefix("gift:") {
        let (gift, count) = rest.split_once(',').unwrap_or((rest, "1"));
        let gift = gift.parse::<u32>().unwrap_or(1);
        let count = count.parse::<u32>().unwrap_or(1);
        send_gift(room, user, gift, count);
        return;
    }
    if let Some(n) = action.strip_prefix("pk:") {
        if let Ok(points) = n.parse::<u32>() {
            pk_add(room, 0, points);
        }
        return;
    }
    let _ = send_chat(room, user, action);
}

pub fn gift_ranking(room: &str) -> Vec<(String, u64)> {
    gifts().lock().unwrap_or_else(|e| e.into_inner()).ranking(room)
}

pub fn gift_ranking_value(room: &str) -> Vec<(String, u64)> {
    gifts().lock().unwrap_or_else(|e| e.into_inner()).ranking_value(room)
}

pub fn gift_ranking_period(room: &str, period_ms: u64) -> Vec<(String, u64)> {
    let since = now_ms().saturating_sub(period_ms);
    gifts().lock().unwrap_or_else(|e| e.into_inner()).ranking_since(room, since)
}

pub fn gift_events(room: &str, limit: usize) -> Vec<GiftEvent> {
    gifts().lock().unwrap_or_else(|e| e.into_inner()).recent_events(room, limit)
}

fn event_log_path() -> std::path::PathBuf {
    std::env::var("M7S_GIFT_EVENT_LOG")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("data/gift-events.jsonl"))
}

pub fn gift_events_period(room: &str, period_ms: u64, limit: usize) -> Vec<GiftEvent> {
    let since = now_ms().saturating_sub(period_ms);
    gifts().lock().unwrap_or_else(|e| e.into_inner()).events_since(room, since, limit)
}

fn trim_jsonl(path: &std::path::Path, max: usize) {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= max {
        return;
    }
    let keep = lines[lines.len() - max..].join("\n") + "\n";
    let _ = std::fs::write(path, keep);
}
