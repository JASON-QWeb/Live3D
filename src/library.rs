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
    #[serde(default)]
    pub source_ext: String,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct BgmTrack {
    pub id: Uuid,
    pub title: String,
    pub duration_ms: usize,
    #[serde(default)]
    pub source_ext: String,
}

pub fn list(app: &App) -> Result<Vec<Track>> {
    let file = app.dir.join("tracks.json");
    if file.exists() {
        config::read_json(&file)
    } else {
        Ok(Vec::new())
    }
}
fn pcm_path(app: &App, id: Uuid) -> PathBuf {
    app.dir.join("tracks").join(format!("{id}.pcm"))
}
fn source_path(app: &App, id: Uuid, ext: &str) -> PathBuf {
    app.dir.join("tracks").join(format!("{id}.{ext}"))
}
pub async fn add(app: &App, title: &str, ext_hint: Option<&str>, bytes: &[u8]) -> Result<Track> {
    let title = title.trim();
    ensure!(
        !title.is_empty() && title.chars().count() <= 100 && !title.chars().any(char::is_control),
        "曲目名称须在 1 至 100 字内"
    );
    {
        // Fast pre-check before ffmpeg decode so re-importing folders skips existing tracks quickly.
        let _g = app.library_gate.lock().await;
        let tracks = list(app)?;
        ensure!(!tracks.iter().any(|t| t.title == title), "曲目已存在");
        ensure!(tracks.len() < 500, "曲库已达 500 首上限");
    }
    let pcm = audio::decode(bytes).await?;
    let detected_ext = audio::detect_audio_ext(bytes, ext_hint);
    let _g = app.library_gate.lock().await;
    let mut tracks = list(app)?;
    ensure!(!tracks.iter().any(|t| t.title == title), "曲目已存在");
    ensure!(tracks.len() < 500, "曲库已达 500 首上限");
    let t = Track {
        id: Uuid::new_v4(),
        title: title.into(),
        duration_ms: pcm.len() / 48,
        source_ext: detected_ext.unwrap_or("").into(),
    };
    config::prepare_dir(&app.dir.join("tracks"))?;
    let p_file = pcm_path(app, t.id);
    config::atomic(&p_file, &pcm)?;
    let s_file = (!t.source_ext.is_empty()).then(|| source_path(app, t.id, &t.source_ext));
    if let Some(ref sf) = s_file {
        if let Err(e) = config::atomic(sf, bytes) {
            let _ = std::fs::remove_file(&p_file);
            return Err(e);
        }
    }
    tracks.push(t.clone());
    if let Err(e) = config::write_json(&app.dir.join("tracks.json"), &tracks) {
        let _ = std::fs::remove_file(&p_file);
        if let Some(sf) = s_file {
            let _ = std::fs::remove_file(sf);
        }
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
    let removed = tracks.remove(index);
    config::write_json(&app.dir.join("tracks.json"), &tracks)?;
    let p = pcm_path(app, id);
    if p.exists() {
        let _ = std::fs::remove_file(p);
    }
    if !removed.source_ext.is_empty() {
        let s = source_path(app, id, &removed.source_ext);
        if s.exists() {
            let _ = std::fs::remove_file(s);
        }
    }
    Ok(())
}
pub async fn clear(app: &App) -> Result<usize> {
    let _g = app.library_gate.lock().await;
    let tracks = list(app)?;
    let count = tracks.len();
    config::write_json(&app.dir.join("tracks.json"), &Vec::<Track>::new())?;
    for t in tracks {
        let p = pcm_path(app, t.id);
        if p.exists() {
            let _ = std::fs::remove_file(p);
        }
        if !t.source_ext.is_empty() {
            let s = source_path(app, t.id, &t.source_ext);
            if s.exists() {
                let _ = std::fs::remove_file(s);
            }
        }
    }
    Ok(count)
}
pub async fn audio(app: &App, id: Uuid) -> Result<Bytes> {
    Ok(tokio::fs::read(pcm_path(app, id))
        .await
        .context("曲目已移除或无法读取")?
        .into())
}
pub async fn playback_audio(app: &App, id: Uuid) -> Result<(&'static str, Bytes)> {
    let track = list(app)?
        .into_iter()
        .find(|t| t.id == id)
        .context("找不到曲目")?;
    if !track.source_ext.is_empty() {
        let p = source_path(app, id, &track.source_ext);
        if let Ok(raw) = tokio::fs::read(&p).await {
            return Ok((audio::mime_for_ext(&track.source_ext), raw.into()));
        }
    }
    let pcm = audio(app, id).await?;
    Ok(("audio/wav", audio::wav(&pcm).into()))
}

// =========================================================================
// Background Music (BGM) Library — Stage-only stereo playback (never drives Vidu)
// =========================================================================

pub fn bgm_list(app: &App) -> Result<Vec<BgmTrack>> {
    let file = app.dir.join("bgm.json");
    if file.exists() {
        config::read_json(&file)
    } else {
        Ok(Vec::new())
    }
}
fn bgm_path(app: &App, id: Uuid, ext: &str) -> PathBuf {
    app.dir.join("bgm").join(format!("{id}.{ext}"))
}
pub async fn bgm_add(
    app: &App,
    title: &str,
    ext_hint: Option<&str>,
    bytes: &[u8],
) -> Result<BgmTrack> {
    let title = title.trim();
    ensure!(
        !title.is_empty() && title.chars().count() <= 100 && !title.chars().any(char::is_control),
        "背景音乐名称须在 1 至 100 字内"
    );
    {
        let _g = app.library_gate.lock().await;
        let items = bgm_list(app)?;
        ensure!(!items.iter().any(|t| t.title == title), "曲目已存在");
        ensure!(items.len() < 500, "背景音乐已达 500 首上限");
    }
    let pcm = audio::decode(bytes).await?;
    let detected = audio::detect_audio_ext(bytes, ext_hint);
    let (ext, stored): (&str, Vec<u8>) = match detected {
        Some(e) => (e, bytes.to_vec()),
        None => ("wav", audio::wav(&pcm)),
    };
    let _g = app.library_gate.lock().await;
    let mut items = bgm_list(app)?;
    ensure!(!items.iter().any(|t| t.title == title), "曲目已存在");
    ensure!(items.len() < 500, "背景音乐已达 500 首上限");
    let t = BgmTrack {
        id: Uuid::new_v4(),
        title: title.into(),
        duration_ms: pcm.len() / 48,
        source_ext: ext.into(),
    };
    config::prepare_dir(&app.dir.join("bgm"))?;
    let file = bgm_path(app, t.id, ext);
    config::atomic(&file, &stored)?;
    items.push(t.clone());
    if let Err(e) = config::write_json(&app.dir.join("bgm.json"), &items) {
        let _ = std::fs::remove_file(file);
        return Err(e);
    }
    Ok(t)
}
pub async fn bgm_remove(app: &App, id: Uuid) -> Result<()> {
    let _g = app.library_gate.lock().await;
    let mut items = bgm_list(app)?;
    let index = items
        .iter()
        .position(|t| t.id == id)
        .context("找不到背景音乐")?;
    let removed = items.remove(index);
    config::write_json(&app.dir.join("bgm.json"), &items)?;
    let ext = if removed.source_ext.is_empty() {
        "wav"
    } else {
        removed.source_ext.as_str()
    };
    let p = bgm_path(app, id, ext);
    if p.exists() {
        let _ = std::fs::remove_file(p);
    }
    Ok(())
}
pub async fn bgm_clear(app: &App) -> Result<usize> {
    let _g = app.library_gate.lock().await;
    let items = bgm_list(app)?;
    let count = items.len();
    config::write_json(&app.dir.join("bgm.json"), &Vec::<BgmTrack>::new())?;
    for t in items {
        let ext = if t.source_ext.is_empty() {
            "wav"
        } else {
            t.source_ext.as_str()
        };
        let p = bgm_path(app, t.id, ext);
        if p.exists() {
            let _ = std::fs::remove_file(p);
        }
    }
    Ok(count)
}
pub async fn bgm_audio(app: &App, id: Uuid) -> Result<(&'static str, Bytes)> {
    let item = bgm_list(app)?
        .into_iter()
        .find(|t| t.id == id)
        .context("找不到背景音乐")?;
    let ext = if item.source_ext.is_empty() {
        "wav"
    } else {
        item.source_ext.as_str()
    };
    let raw = tokio::fs::read(bgm_path(app, id, ext))
        .await
        .context("背景音乐已移除或无法读取")?;
    Ok((audio::mime_for_ext(ext), raw.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_track_json_deserializes_with_empty_source_ext() {
        let json = r#"[{"id":"11111111-1111-4111-8111-111111111111","title":"云烟成雨","duration_ms":215000}]"#;
        let tracks: Vec<Track> = serde_json::from_str(json).unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].title, "云烟成雨");
        assert!(tracks[0].source_ext.is_empty());
    }
}
