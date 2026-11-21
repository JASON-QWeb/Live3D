use crate::{
    audience,
    audio::checked,
    config::{self, key, now},
    speech::{self, Offer},
    state::{App, View, lock},
    vidu,
    wire::{self, Command, Packet},
};
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize},
    },
    time::Duration,
};
use tokio::{
    sync::{mpsc, watch},
    time::sleep,
};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingGift {
    pub author: Option<String>,
    pub gift_name: String,
    pub count: u32,
}

pub struct Run {
    pub stop: CancellationToken,
    pub epoch: watch::Sender<u64>,
    pub wire: mpsc::Sender<Packet>,
    pub jobs: mpsc::Sender<Offer>,
    pub gifts: mpsc::Sender<Offer>,
    pub choreo: watch::Sender<bool>,
    pub next_action: AtomicU64,
    pub history: Mutex<std::collections::VecDeque<(String, String)>>,
    pub singing: AtomicBool,
    pub busy: AtomicBool,
    pub last_activity_ms: AtomicU64,
    pub last_song_end_ms: AtomicU64,
    pub last_ambient_ms: AtomicU64,
    pub last_join_welcome_ms: AtomicU64,
    pub song_gifts: Mutex<Vec<PendingGift>>,
    pub next_track_index: AtomicUsize,
    pub next_welcome_index: AtomicUsize,
}
pub fn signal(app: &App, t: u8, p: Value) -> Result<()> {
    let r = app.active()?;
    let epoch = *r.epoch.borrow();
    r.wire
        .try_send(Packet {
            epoch,
            cmd: Command::Signal(t, p),
        })
        .context("信令队列已满或已关闭")?;
    Ok(())
}
pub async fn start(app: &App) -> Result<()> {
    let _g = app.gate.lock().await;
    ensure!(!app.quit.is_cancelled(), "Avatar3D 正在退出");
    ensure!(lock(&app.run).is_none(), "已有会话正在运行");
    ensure!(
        !app.dir.join("open-session.json").exists(),
        "请先核对上一场连线是否结束"
    );
    let k = key("vidu")?.context("请先在设置中保存 Vidu 密钥")?;
    let cfg = app.config();
    let body = vidu::create_body(app, &cfg)?;
    app.journal(&json!({"started":now(),"id":null}))?;
    let (wire, rx) = mpsc::channel(16);
    let (jobs, jrx) = mpsc::channel(24);
    let (epoch, _) = watch::channel(0);
    let (gifts, grx) = mpsc::channel(12);
    let (choreo, _) = watch::channel(false);
    let run = Arc::new(Run {
        stop: CancellationToken::new(),
        epoch,
        wire,
        jobs,
        gifts,
        choreo,
        next_action: 0.into(),
        history: Default::default(),
        singing: AtomicBool::new(false),
        busy: AtomicBool::new(false),
        last_activity_ms: AtomicU64::new(0),
        last_song_end_ms: AtomicU64::new(0),
        last_ambient_ms: AtomicU64::new(0),
        last_join_welcome_ms: AtomicU64::new(0),
        song_gifts: Mutex::new(Vec::new()),
        next_track_index: AtomicUsize::new(0),
        next_welcome_index: AtomicUsize::new(0),
    });
    *lock(&app.run) = Some(run.clone());
    lock(&app.seen).clear();
    lock(&app.screen).tone = None;
    *lock(&app.view) = View {
        phase: "starting".into(),
        message: "正在创建云端会话".into(),
        started: now(),
        ..View::default()
    };
    let core = app.clone();
    let active = run.clone();
    tokio::spawn(async move {
        let worker = tokio::spawn(speech::worker(core.clone(), active.clone(), jrx, grx));
        let schedule = tokio::spawn(crate::choreo::worker(core.clone(), active.clone()));
        let director = tokio::spawn(crate::director::worker(core.clone(), active.clone()));
        let timer = active.stop.clone();
        let limit = tokio::spawn(async move {
            sleep(Duration::from_secs(cfg.max_minutes as u64 * 60)).await;
            timer.cancel();
        });
        if let Err(e) = lifecycle(&core, &active, &k, body, rx).await {
            let unresolved = core.dir.join("open-session.json").exists();
            core.phase(
                if unresolved { "unconfirmed" } else { "error" },
                &e.to_string(),
            );
            core.event("连线", e.to_string());
        }
        active.stop.cancel();
        limit.abort();
        let _ = worker.await;
        let _ = schedule.await;
        let _ = director.await;
        let _g = core.gate.lock().await;
        lock(&core.screen).rtc = None;
        {
            let mut v = lock(&core.view);
            v.subtitle.clear();
            v.ended = now();
            v.pending = 0;
            v.choreo = false;
            v.voice_busy = false;
            v.singing = false;
            v.current_song.clear();
        }
        *lock(&core.run) = None;
    });
    Ok(())
}
async fn lifecycle(
    app: &App,
    r: &Arc<Run>,
    k: &str,
    body: Value,
    mut rx: mpsc::Receiver<Packet>,
) -> Result<()> {
    let raw = vidu::create(app, k, body)
        .await
        .context("创建结果未确认，请核对 Vidu 控制台")?;
    if raw.status().is_client_error() && raw.status().as_u16() != 408 {
        fs::remove_file(app.dir.join("open-session.json"))?;
        checked(raw, "Vidu").await?;
        unreachable!()
    }
    let created = vidu::created(raw).await?;
    let id = created.id;
    let started = lock(&app.view).started;
    lock(&app.view).live_id = id.clone();
    if let Err(e) = app.journal(&json!({"started":started,"id":id})) {
        let _ = wire::request_end(&id, k).await;
        return Err(e.context("会话日志写入失败，已请求关闭"));
    }
    let usable = created.rtc.is_some();
    lock(&app.screen).rtc = created.rtc;
    let mut cause = String::new();
    if usable && !r.stop.is_cancelled() {
        let collector = tokio::spawn(audience::youtube(app.clone(), r.stop.clone()));
        cause = connected(app, r, &id, k, &mut rx).await;
        collector.abort();
    } else if !usable {
        cause = "未收到有效的腾讯 RTC 观看凭证".into();
    }
    r.stop.cancel();
    lock(&app.screen).rtc = None;
    app.phase("stopping", "正在核对云端结束状态");
    if !cause.is_empty() {
        app.event("连线", cause);
    }
    finish(app, k, &id).await
}
async fn connected(
    app: &App,
    r: &Arc<Run>,
    id: &str,
    k: &str,
    rx: &mut mpsc::Receiver<Packet>,
) -> String {
    for attempt in 0..3 {
        if r.stop.is_cancelled() {
            break;
        }
        if attempt > 0 {
            app.phase("reconnecting", "正在恢复同一场会话");
            speech::interrupt(app, r);
            lock(&app.view).metrics.reconnects += 1;
            let delay = Duration::from_secs(attempt);
            tokio::select! {
                _ = r.stop.cancelled() => break,
                _ = sleep(delay) => (),
            }
            if let Ok(s) = vidu::query(app, k, id, false).await
                && ended(&s)
            {
                break;
            }
        }
        match wire::serve(app, r, id, k, rx).await {
            Ok(()) => return String::new(),
            Err(e) => {
                app.event("信令", format!("连接中断：{e}"));
                if attempt == 2 {
                    return "信令恢复失败，正在结束云端会话".into();
                }
            }
        }
    }
    String::new()
}
fn ended(v: &Value) -> bool {
    v.pointer("/live/status").and_then(Value::as_str) == Some("ended")
}
fn number(v: &Value) -> Option<f64> {
    v.as_f64().or_else(|| v.as_str()?.parse().ok())
}
fn save_report(app: &App, id: &str, v: &Value) -> Result<()> {
    let s = &v["live"];
    let mut view = lock(&app.view);
    view.metrics.billed_seconds = number(&s["billed_seconds"]).map(|x| x as u64);
    view.metrics.credits = number(&s["credits_cost"]);
    let report = json!({"id":id,"started":view.started,"ended":now(),"cloud_status":s["status"],
        "metrics":view.metrics,"recording":app.config().recording});
    drop(view);
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(app.dir.join("sessions.jsonl"))?;
    writeln!(f, "{}", report)?;
    f.sync_data()?;
    fs::remove_file(app.dir.join("open-session.json"))?;
    app.phase("ended", "Vidu 已确认会话结束");
    app.event("连线", "Vidu 已确认会话结束");
    Ok(())
}
pub async fn finish(app: &App, k: &str, id: &str) -> Result<()> {
    for i in 0..6 {
        if i > 0 {
            sleep(Duration::from_secs(2)).await;
        }
        if let Ok(v) = vidu::query(app, k, id, false).await {
            if ended(&v) {
                return save_report(app, id, &v);
            }
            if v.pointer("/live/status").and_then(Value::as_str) == Some("ending") {
                continue;
            }
        }
        if i == 0 || i == 3 {
            let _ = wire::request_end(id, k).await;
        }
    }
    bail!("云端尚未确认结束。请重试核对，或打开 Vidu 控制台检查该场次")
}
pub async fn recover(app: &App, manual: bool) -> Result<()> {
    let _g = app.gate.lock().await;
    ensure!(lock(&app.run).is_none(), "会话仍在处理中");
    let p = app.dir.join("open-session.json");
    let j: Value = config::read_json(&p).context("没有待核对的会话")?;
    if manual {
        let path = app.dir.join(format!("operator-confirmed-{}.json", now()));
        fs::rename(p, path)?;
        app.phase("idle", "操作员已在控制台核对并解除锁定");
        return Ok(());
    }
    let id = j["id"]
        .as_str()
        .context("创建响应缺少场次 ID，请在 Vidu 控制台核对此时段")?;
    lock(&app.view).live_id = id.to_owned();
    lock(&app.view).started = j["started"].as_u64().unwrap_or_default();
    let k = key("vidu")?.context("请填写本场使用的 Vidu 密钥")?;
    finish(app, &k, id).await
}
pub fn stop(app: &App) -> Result<()> {
    let r = lock(&app.run).clone().context("当前没有云端会话")?;
    app.phase("stopping", "正在结束云端会话");
    r.stop.cancel();
    Ok(())
}
