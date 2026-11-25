//! FLV recording: append published FLV tags to a single file per stream.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::config::RecordConfig;
use crate::flv::flv_header;
use crate::stream::StreamManager;
use crate::time::now_ms;

pub struct RecordManager {
    cfg: RecordConfig,
    streams: Arc<StreamManager>,
    active: Arc<Mutex<HashSet<String>>>,
}

impl RecordManager {
    pub fn new(streams: Arc<StreamManager>, cfg: RecordConfig) -> Arc<Self> {
        Arc::new(Self {
            cfg,
            streams,
            active: Arc::new(Mutex::new(HashSet::new())),
        })
    }

    pub fn ensure(self: &Arc<Self>, app: &str, name: &str) {
        if !self.cfg.enabled {
            return;
        }
        let key = format!("{app}/{name}");
        {
            let mut active = self.active.lock().unwrap_or_else(|e| e.into_inner());
            if !active.insert(key.clone()) {
                return;
            }
        }
        let Some(stream) = self.streams.get(app, name) else {
            self.active
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&key);
            return;
        };
        let dir = PathBuf::from(&self.cfg.dir).join(app);
        if let Err(e) = fs::create_dir_all(&dir) {
            log::warn!("record dir {}: {e}", dir.display());
            return;
        }
        let path = dir.join(format!("{name}-{}.flv", now_ms()));
        let active = self.active.clone();
        tokio::spawn(async move {
            if let Err(e) = record_task(stream, path).await {
                log::warn!("record failed: {e:#}");
            }
            active
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&key);
        });
    }
}

async fn record_task(stream: Arc<crate::stream::Stream>, path: PathBuf) -> anyhow::Result<()> {
    let (cache, mut rx) = stream.subscribe();
    let mut file = File::create(&path)?;
    file.write_all(&flv_header(true, true))?;
    for tag in cache {
        file.write_all(&tag.to_bytes())?;
    }
    file.flush()?;
    loop {
        match rx.recv().await {
            Ok(tag) => {
                file.write_all(&tag.to_bytes())?;
                file.flush()?;
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(_) => break,
        }
    }
    stream.leave();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flv::FlvTag;

    #[test]
    fn records_flv() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let dir = std::env::current_dir()
                .unwrap()
                .join("target")
                .join(format!("test-record-{}", now_ms()));
            let streams = StreamManager::new(10);
            let stream = streams.get_or_create("live", "test");
            stream.push(Arc::new(FlvTag::new(9, 0, vec![0x17, 1, 2, 3])));
            let manager = RecordManager::new(
                streams.clone(),
                RecordConfig {
                    enabled: true,
                    dir: dir.to_string_lossy().to_string(),
                },
            );
            manager.ensure("live", "test");
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            let path = fs::read_dir(dir.join("live"))
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path();
            let bytes = fs::read(path).unwrap();
            assert_eq!(&bytes[0..3], b"FLV");
            assert!(bytes.len() > 13);
            let _ = fs::remove_dir_all(dir);
        });
    }
}
