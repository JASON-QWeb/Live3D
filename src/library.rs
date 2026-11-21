use crate::{audio, config, state::App};
use anyhow::{Context, Result, ensure};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Clone, Deserialize, Serialize)]
pub struct Track {
    pub id: Uuid,
    pub title: String,
    pub duration_ms: usize,
}
pub fn list(app: &App) -> Result<Vec<Track>> {
    let file = app.dir.join("tracks.json");
    if file.exists() {
        config::read_json(&file)
    } else {
        Ok(Vec::new())
    }
}
fn path(app: &App, id: Uuid) -> PathBuf {
    app.dir.join("tracks").join(format!("{id}.pcm"))
}
pub async fn add(app: &App, title: &str, bytes: &[u8]) -> Result<Track> {
    let title = title.trim();
    ensure!(
        !title.is_empty() && title.chars().count() <= 100 && !title.chars().any(char::is_control),
        "曲目名称须在 1 至 100 字内"
    );
    let pcm = audio::decode(bytes).await?;
    let _g = app.library_gate.lock().await;
    let mut tracks = list(app)?;
    ensure!(
        tracks.len() < 500 && !tracks.iter().any(|t| t.title == title),
        "曲目名称重复或已达 500 首"
    );
    let t = Track {
        id: Uuid::new_v4(),
        title: title.into(),
        duration_ms: pcm.len() / 48,
    };
    config::prepare_dir(&app.dir.join("tracks"))?;
    let file = path(app, t.id);
    config::atomic(&file, &pcm)?;
    tracks.push(t.clone());
    if let Err(e) = config::write_json(&app.dir.join("tracks.json"), &tracks) {
        let _ = std::fs::remove_file(file);
        return Err(e);
    }
    Ok(t)
}
pub async fn remove(app: &App, id: Uuid) -> Result<()> {
    let _g = app.library_gate.lock().await;
    let mut tracks = list(app)?;
    let index = tracks
        .iter()
        .position(|t| t.id == id)
        .context("找不到曲目")?;
    tracks.remove(index);
    config::write_json(&app.dir.join("tracks.json"), &tracks)?;
    let p = path(app, id);
    if p.exists() {
        std::fs::remove_file(p)?;
    }
    Ok(())
}
pub async fn audio(app: &App, id: Uuid) -> Result<Bytes> {
    Ok(tokio::fs::read(path(app, id))
        .await
        .context("曲目已移除或无法读取")?
        .into())
}
