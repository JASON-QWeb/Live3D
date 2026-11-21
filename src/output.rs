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
    let (rtc, check) = {
        let mut s = lock(&app.screen);
        if s.lease != q.client && time.saturating_sub(s.seen) < 10_000 {
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
        )
    };
    let subtitles = app.config().subtitles;
    let mut v = lock(&app.view);
    let current = q.live_id.as_deref() == Some(v.live_id.as_str()) && !v.live_id.is_empty();
    v.output = status.into();
    if q.status == "playing" && current && q.width.unwrap_or(0) > 0 {
        if v.metrics.first_frame_ms.is_none() {
            v.metrics.first_frame_ms = Some(time.saturating_sub(v.started));
        }
        v.metrics.output_width = q.width;
        v.metrics.output_height = q.height;
        v.metrics.output_fps = q.fps.filter(|n| n.is_finite() && (0.0..=240.0).contains(n));
    }
    let text = if subtitles && time < v.subtitle_until {
        v.subtitle.as_str()
    } else {
        ""
    };
    Ok(
        json!({"owner":true,"live_id":v.live_id,"check":check,"rtc":rtc,"subtitle":text,"phase":v.phase}),
    )
}
