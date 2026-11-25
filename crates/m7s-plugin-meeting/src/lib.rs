//! Meeting room state (join/leave/publish).
use std::collections::{HashMap, HashSet};
#[derive(Default)]
pub struct Meeting {
    rooms: HashMap<String, HashSet<String>>,
    publishing: HashSet<(String, String)>,
    muted: HashSet<(String, String)>,
    hands: HashSet<(String, String)>,
}
impl Meeting {
    pub fn join(&mut self, room: &str, user: &str) { self.rooms.entry(room.into()).or_default().insert(user.into()); }
    pub fn leave(&mut self, room: &str, user: &str) {
        if let Some(r)=self.rooms.get_mut(room) { r.remove(user); }
        self.publishing.remove(&(room.into(), user.into()));
    }
    pub fn publish(&mut self, room: &str, user: &str, on: bool) {
        let k=(room.into(), user.into());
        if on { self.publishing.insert(k); } else { self.publishing.remove(&k); }
    }
    pub fn set_muted(&mut self, room: &str, user: &str, muted: bool) {
        let k = (room.to_string(), user.to_string());
        if muted { self.muted.insert(k); } else { self.muted.remove(&k); }
    }
    pub fn is_muted(&self, room: &str, user: &str) -> bool {
        self.muted.contains(&(room.to_string(), user.to_string()))
    }
    pub fn raise_hand(&mut self, room: &str, user: &str, up: bool) {
        let k = (room.to_string(), user.to_string());
        if up { self.hands.insert(k); } else { self.hands.remove(&k); }
    }
    pub fn hands(&self, room: &str) -> usize {
        self.hands.iter().filter(|(r, _)| r == room).count()
    }
    pub fn kick(&mut self, room: &str, user: &str) {
        self.leave(room, user);
        self.muted.remove(&(room.to_string(), user.to_string()));
        self.hands.remove(&(room.to_string(), user.to_string()));
    }
    pub fn members(&self, room: &str) -> usize { self.rooms.get(room).map(|r| r.len()).unwrap_or(0) }
    pub fn publishers(&self, room: &str) -> usize { self.publishing.iter().filter(|(r,_)| r==room).count() }
}
#[cfg(test)] mod tests { use super::*; #[test] fn publish() { let mut m=Meeting::default(); m.join("r","a"); m.publish("r","a",true); assert_eq!(m.publishers("r"),1); m.leave("r","a"); assert_eq!(m.publishers("r"),0); } }

static MEETING: std::sync::OnceLock<std::sync::Mutex<Meeting>> = std::sync::OnceLock::new();

fn meeting() -> &'static std::sync::Mutex<Meeting> {
    MEETING.get_or_init(|| std::sync::Mutex::new(Meeting::default()))
}

pub fn join_room(room: &str, user: &str) {
    meeting().lock().unwrap_or_else(|e| e.into_inner()).join(room, user);
}
pub fn leave_room(room: &str, user: &str) {
    meeting().lock().unwrap_or_else(|e| e.into_inner()).leave(room, user);
}
pub fn publish(room: &str, user: &str, on: bool) {
    meeting().lock().unwrap_or_else(|e| e.into_inner()).publish(room, user, on);
}
pub fn room_names() -> Vec<String> {
    meeting().lock().unwrap_or_else(|e| e.into_inner()).rooms.iter().filter(|(_, v)| !v.is_empty()).map(|(k, _)| k.clone()).collect()
}

pub fn members(room: &str) -> usize {
    meeting().lock().unwrap_or_else(|e| e.into_inner()).members(room)
}
pub fn publishers(room: &str) -> usize {
    meeting().lock().unwrap_or_else(|e| e.into_inner()).publishers(room)
}

#[cfg(test)]
mod api_tests {
    use super::*;
    #[test]
    fn join_publish() {
        join_room("m", "a");
        publish("m", "a", true);
        assert_eq!(members("m"), 1);
        assert_eq!(publishers("m"), 1);
        leave_room("m", "a");
        assert_eq!(publishers("m"), 0);
    }
}

static STREAMS: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, String>>> = std::sync::OnceLock::new();

fn streams() -> &'static std::sync::Mutex<std::collections::HashMap<String, String>> {
    STREAMS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

pub fn set_stream(room: &str, stream: &str) {
    streams().lock().unwrap_or_else(|e| e.into_inner()).insert(room.to_string(), stream.to_string());
}

pub fn stream_for(room: &str) -> Option<String> {
    streams().lock().unwrap_or_else(|e| e.into_inner()).get(room).cloned()
}

#[cfg(test)]
mod map_tests {
    use super::*;
    #[test]
    fn maps_room_to_stream() {
        set_stream("mr", "live/cam");
        assert_eq!(stream_for("mr").as_deref(), Some("live/cam"));
        assert_eq!(stream_for("none"), None);
    }
}

pub fn set_muted(room: &str, user: &str, muted: bool) {
    meeting().lock().unwrap_or_else(|e| e.into_inner()).set_muted(room, user, muted);
}
pub fn muted_count(room: &str) -> usize {
    meeting().lock().unwrap_or_else(|e| e.into_inner()).muted.iter().filter(|(r, _)| r == room).count()
}
pub fn is_muted(room: &str, user: &str) -> bool {
    meeting().lock().unwrap_or_else(|e| e.into_inner()).is_muted(room, user)
}
pub fn raise_hand(room: &str, user: &str, up: bool) {
    meeting().lock().unwrap_or_else(|e| e.into_inner()).raise_hand(room, user, up);
}
pub fn hands(room: &str) -> usize {
    meeting().lock().unwrap_or_else(|e| e.into_inner()).hands(room)
}
pub fn kick(room: &str, user: &str) {
    meeting().lock().unwrap_or_else(|e| e.into_inner()).kick(room, user);
}

#[cfg(test)]
mod moderation_tests {
    use super::*;
    #[test]
    fn mute_hand_kick() {
        let room = format!("mod-{}", std::process::id());
        join_room(&room, "a");
        set_muted(&room, "a", true);
        assert!(is_muted(&room, "a"));
        raise_hand(&room, "a", true);
        assert_eq!(hands(&room), 1);
        kick(&room, "a");
        assert_eq!(members(&room), 0);
        assert!(!is_muted(&room, "a"));
        assert_eq!(hands(&room), 0);
    }
}
