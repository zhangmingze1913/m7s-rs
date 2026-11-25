//! Optional SQLite metadata store for streams/rooms.
#![allow(clippy::items_after_test_module)]

use rusqlite::{Connection, params};

pub type StreamRow = (String, String, String, u64, u64);
pub type RoomRow = (String, String, u64, bool);

pub struct MetaDb {
    conn: std::sync::Mutex<Connection>,
}

impl MetaDb {
    pub fn open(path: &std::path::Path) -> Result<Self, String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let conn = Connection::open(path).map_err(|e| e.to_string())?;
        migrate(&conn)?;
        Ok(Self { conn: std::sync::Mutex::new(conn) })
    }
    pub fn schema_version(&self) -> i64 {
        let c = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        c.query_row("PRAGMA user_version", [], |row| row.get(0)).unwrap_or(0)
    }

    pub fn upsert_stream(&self, key: &str, app: &str, name: &str, bytes: u64, subscribers: u64) -> Result<(), String> {
        let c = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        c.execute(
            "INSERT INTO m7s_streams(key,app,name,bytes,subscribers) VALUES(?1,?2,?3,?4,?5)
             ON CONFLICT(key) DO UPDATE SET bytes=?4, subscribers=?5",
            params![key, app, name, bytes as i64, subscribers as i64],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn list_streams(&self) -> Result<Vec<StreamRow>, String> {
        let c = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = c
            .prepare("SELECT key,app,name,bytes,subscribers FROM m7s_streams ORDER BY key")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)? as u64,
                    r.get::<_, i64>(4)? as u64,
                ))
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }

    pub fn upsert_room(&self, name: &str, title: &str, created: u64, closed: bool) -> Result<(), String> {
        let c = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        c.execute(
            "INSERT INTO m7s_rooms(name,title,created,closed) VALUES(?1,?2,?3,?4)
             ON CONFLICT(name) DO UPDATE SET title=?2, closed=?4",
            params![name, title, created as i64, i64::from(closed)],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn list_rooms(&self) -> Result<Vec<RoomRow>, String> {
        let c = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = c
            .prepare("SELECT name,title,created,closed FROM m7s_rooms ORDER BY name")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)? as u64,
                    r.get::<_, i64>(3)? != 0,
                ))
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }
}

static GLOBAL: std::sync::OnceLock<Option<std::sync::Arc<MetaDb>>> = std::sync::OnceLock::new();

/// Open `M7S_META_DB` once; `None` when unset or open fails.
pub fn global() -> Option<&'static std::sync::Arc<MetaDb>> {
    GLOBAL
        .get_or_init(|| {
            let path = std::env::var("M7S_META_DB").ok()?;
            MetaDb::open(std::path::Path::new(&path)).ok().map(std::sync::Arc::new)
        })
        .as_ref()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_migrates_and_is_idempotent() {
        let path = std::env::temp_dir().join(format!("m7s-migrate-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let db = MetaDb::open(&path).unwrap();
        assert_eq!(db.schema_version(), 3);
        drop(db);
        let db = MetaDb::open(&path).unwrap();
        assert_eq!(db.schema_version(), 3);
        db.upsert_stream("live/m", "live", "m", 1, 1).unwrap();
        assert_eq!(db.list_streams().unwrap().len(), 1);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn sqlite_roundtrip() {
        let path = std::env::temp_dir().join(format!("m7s-meta-{}.db", std::process::id()));
        let db = MetaDb::open(&path).unwrap();
        db.upsert_stream("live/a", "live", "a", 100, 2).unwrap();
        db.upsert_stream("live/a", "live", "a", 200, 3).unwrap();
        let streams = db.list_streams().unwrap();
        assert_eq!(streams.len(), 1);
        assert_eq!(streams[0].3, 200);
        assert_eq!(streams[0].4, 3);
        db.upsert_room("r", "Title", 1, false).unwrap();
        db.upsert_room("r", "Title", 1, true).unwrap();
        let rooms = db.list_rooms().unwrap();
        assert_eq!(rooms.len(), 1);
        assert!(rooms[0].3);
        let _ = std::fs::remove_file(path);
    }
}


fn migrate(conn: &Connection) -> Result<(), String> {
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|e| e.to_string())?;
    if version < 1 {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS m7s_streams (
                key TEXT PRIMARY KEY, app TEXT NOT NULL, name TEXT NOT NULL,
                bytes INTEGER NOT NULL DEFAULT 0, subscribers INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS m7s_rooms (
                name TEXT PRIMARY KEY, title TEXT NOT NULL, created INTEGER NOT NULL,
                closed INTEGER NOT NULL DEFAULT 0
            );",
        )
        .map_err(|e| e.to_string())?;
        conn.pragma_update(None, "user_version", 1)
            .map_err(|e| e.to_string())?;
    }
    if version < 2 {
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_m7s_streams_subscribers
                 ON m7s_streams(subscribers);
             CREATE INDEX IF NOT EXISTS idx_m7s_rooms_closed
                 ON m7s_rooms(closed);",
        )
        .map_err(|e| e.to_string())?;
        conn.pragma_update(None, "user_version", 2)
            .map_err(|e| e.to_string())?;
    }
    if version < 3 {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS m7s_kv (
                 key TEXT PRIMARY KEY, value TEXT NOT NULL
             );",
        )
        .map_err(|e| e.to_string())?;
        conn.pragma_update(None, "user_version", 3)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

impl m7s_sdk::MetaStore for MetaDb {
    fn put(&self, key: &str, value: &str) -> Result<(), String> {
        let c = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        c.execute(
            "INSERT INTO m7s_kv(key,value) VALUES(?1,?2)
             ON CONFLICT(key) DO UPDATE SET value=?2",
            params![key, value],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn get(&self, key: &str) -> Result<Option<String>, String> {
        let c = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = c.prepare("SELECT value FROM m7s_kv WHERE key=?1").map_err(|e| e.to_string())?;
        let mut rows = stmt.query(params![key]).map_err(|e| e.to_string())?;
        if let Some(row) = rows.next().map_err(|e| e.to_string())? {
            Ok(Some(row.get(0).map_err(|e| e.to_string())?))
        } else {
            Ok(None)
        }
    }

    fn delete(&self, key: &str) -> Result<(), String> {
        let c = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        c.execute("DELETE FROM m7s_kv WHERE key=?1", params![key]).map_err(|e| e.to_string())?;
        Ok(())
    }
}
