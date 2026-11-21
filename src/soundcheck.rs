use crate::{
    audio,
    config::now,
    state::{App, lock},
};
use anyhow::{Result, ensure};
use bytes::Bytes;
use uuid::Uuid;
pub struct Tone {
    pub id: String,
    pub wav: Bytes,
    pub until: u64,
    pub played: bool,
}
pub async fn send(app: &App, line: &str) -> Result<()> {
    ensure!(lock(&app.run).is_none(), "云端连线时请使用正常播出指令");
    ensure!(
        now().saturating_sub(lock(&app.screen).seen) < 10_000,
        "请先打开独立舞台或选中 OBS 场景"
    );
    let settings = app.config();
    ensure!(
        settings.tts_mode != "system",
        "舞台试音请选择 MLX 或授权语音服务。系统语音可在控制台本机试听"
    );
    let pcm = audio::synth(&app.http, &settings, line).await?;
    let _g = app.gate.lock().await;
    ensure!(lock(&app.run).is_none(), "已开始云端连线，本次试音已取消");
    let duration = pcm.len() / 48;
    let tone = Tone {
        id: Uuid::new_v4().to_string(),
        wav: audio::wav(&pcm).into(),
        until: now() + duration as u64 + 30_000,
        played: false,
    };
    lock(&app.screen).tone = Some(tone);
    app.event(
        "试音",
        format!("已发送真实语音至舞台（{:.2} 秒）", duration as f64 / 1000.0),
    );
    Ok(())
}
pub fn read(app: &App, id: &str) -> Option<Bytes> {
    let s = lock(&app.screen);
    let t = s.tone.as_ref()?;
    (t.id == id && t.until >= now() && s.rtc.is_none()).then(|| t.wav.clone())
}
pub fn receipt(app: &App, id: &str, client: &str) -> Result<()> {
    let mut s = lock(&app.screen);
    ensure!(s.lease == client, "输出端已切换");
    let t = s
        .tone
        .as_mut()
        .filter(|t| t.id == id)
        .ok_or_else(|| anyhow::anyhow!("试音已结束"))?;
    let first = !t.played;
    t.played = true;
    drop(s);
    if first {
        app.event("试音", "舞台已播放完本次试音");
    }
    Ok(())
}
