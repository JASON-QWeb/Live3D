use crate::{
    config::now,
    state::{App, lock},
};
use anyhow::{Result, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
#[derive(Deserialize)]
pub struct Report {
    pub client: String,
    pub status: String,
    pub live_id: Option<String>,
    pub fps: Option<f64>,
    pub width: Option<u64>,
    pub height: Option<u64>,
    pub rtc_supported: Option<bool>,
    #[serde(default)]
    pub force: bool,
    #[serde(default)]
    pub bgm_status: Option<String>,
}
pub fn bgm_snapshot(app: &App) -> Value {
    let cfg = app.config();
    let items = crate::library::bgm_list(app).unwrap_or_default();
    let (enabled, index, requested, revision, singing, voice_busy) = {
        let s = lock(&app.screen);
        let v = lock(&app.view);
        (
            s.bgm_enabled && (!items.is_empty() || s.requested_bgm.is_some()),
            if items.is_empty() {
                0
            } else {
                s.bgm_index % items.len()
            },
            s.requested_bgm.clone(),
            s.bgm_revision,
            v.singing,
            v.voice_busy,
        )
    };
    let current = requested.as_ref().or(items.get(index));
    {
        let mut v = lock(&app.view);
        v.bgm_playing = enabled && !singing;
        v.current_bgm = if enabled {
            current.map(|t| t.title.clone()).unwrap_or_default()
        } else {
            String::new()
        };
    }
    json!({
        "enabled": enabled,
        "paused_for_song": singing,
        "ducked": voice_busy && !singing,
        "volume": cfg.bgm_volume as f32 / 100.0,
        "duck_percent": cfg.bgm_duck_percent as f32 / 100.0,
        "revision": revision,
        "index": index,
        "online": current.is_some_and(|t| t.online.is_some()),
        "track_id": current.map(|t| t.id.to_string()),
        "title": current.map(|t| t.title.as_str()).unwrap_or(""),
        "queue": items.iter().map(|t| json!({"id": t.id.to_string(), "title": t.title})).collect::<Vec<_>>(),
    })
}
pub fn heartbeat(app: &App, q: Report) -> Result<Value> {
    ensure!(
        q.client.len() >= 16 && q.client.len() <= 80,
        "输出端标识无效"
    );
    let status = match q.status.as_str() {
        "standby" => match q.rtc_supported {
            Some(true) => "舞台就绪 · WebRTC / H.264 可用",
            Some(false) => "舞台已连接，浏览器未通过 RTC 检测",
            None => "已连接，待播",
        },
        "joining" => "正在订阅",
        "waiting" => "等待远端画面",
        "playing" => "正在输出画面",
        "audio_blocked" => "浏览器等待允许播放",
        "sdk_error" => "RTC SDK 加载失败",
        "rtc_error" => "RTC 播放失败",
        "lost" => "远端画面中断",
        "released" => "",
        _ => anyhow::bail!("输出端状态无效"),
    };
    let time = now();
    let (rtc, check, song) = {
        let mut s = lock(&app.screen);
        if !q.force && s.lease != q.client && time.saturating_sub(s.seen) < 10_000 {
            return Ok(json!({"owner":false}));
        }
        if q.status == "released" {
            s.seen = 0;
            s.lease.clear();
        } else {
            s.lease = q.client;
            s.seen = time;
        }
        (
            s.rtc.clone(),
            s.tone
                .as_ref()
                .filter(|t| t.until >= time && !t.played && s.rtc.is_none())
                .map(|t| t.id.clone()),
            s.song.clone(),
        )
    };
    let cfg = app.config();
    let (live_id, phase) = {
        let mut v = lock(&app.view);
        let current = q.live_id.as_deref() == Some(v.live_id.as_str()) && !v.live_id.is_empty();
        v.output = status.into();
        v.bgm_status = q.bgm_status.unwrap_or_default();
        if q.status == "playing" && current && q.width.unwrap_or(0) > 0 {
            if v.metrics.first_frame_ms.is_none() {
                v.metrics.first_frame_ms = Some(time.saturating_sub(v.started));
            }
            v.metrics.output_width = q.width;
            v.metrics.output_height = q.height;
            v.metrics.output_fps = q.fps.filter(|n| n.is_finite() && (0.0..=240.0).contains(n));
        }
        (v.live_id.clone(), v.phase.clone())
    };
    let song_json = song.map(|s| {
        let elapsed_ms = if s.state == "play" && s.started_at > 0 {
            time.saturating_sub(s.started_at)
        } else {
            0
        };
        json!({
            "playback_id": s.playback_id,
            "track_id": s.track_id,
            "title": s.title,
            "state": s.state,
            "elapsed_ms": elapsed_ms,
            "volume": cfg.song_volume as f32 / 100.0,
        })
    });
    let bgm = bgm_snapshot(app);
    Ok(json!({
        "owner": true,
        "boot_id": app.boot_id,
        "live_id": live_id,
        "check": check,
        "rtc": rtc,
        "subtitle": "",
        "phase": phase,
        "song": song_json,
        "dance": lock(&app.screen).dance.clone(),
        "bgm": bgm,
    }))
}
