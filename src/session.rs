use crate::model::Session;
use anyhow::{Context, Result};
use std::path::PathBuf;

pub fn root() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("receipt")
}

pub fn sessions_dir() -> PathBuf {
    root().join("sessions")
}

pub fn new_id() -> String {
    let now = chrono::Utc::now();
    format!(
        "{}-{:04x}",
        now.format("%Y%m%d-%H%M%S"),
        now.timestamp_subsec_nanos() & 0xffff
    )
}

pub fn save(s: &Session) -> Result<PathBuf> {
    let dir = sessions_dir().join(&s.id);
    std::fs::create_dir_all(&dir)?;
    let p = dir.join("session.json");
    std::fs::write(&p, serde_json::to_vec_pretty(s)?)?;
    Ok(p)
}

pub fn load(id: &str) -> Result<Session> {
    let p = sessions_dir().join(id).join("session.json");
    let b = std::fs::read(&p).with_context(|| format!("no session {}", id))?;
    Ok(serde_json::from_slice(&b)?)
}

pub fn latest() -> Result<Session> {
    let mut ids: Vec<String> = std::fs::read_dir(sessions_dir())
        .context("no sessions yet")?
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    ids.sort();
    let id = ids.last().context("no sessions yet")?.clone();
    load(&id)
}

pub fn list() -> Result<Vec<Session>> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(sessions_dir()) {
        for e in rd.flatten() {
            if let Ok(b) = std::fs::read(e.path().join("session.json")) {
                if let Ok(s) = serde_json::from_slice::<Session>(&b) {
                    out.push(s);
                }
            }
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}
