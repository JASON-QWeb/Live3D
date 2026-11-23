//! Douyin live comments.
//!
//! By default Avatar3D connects to the live room itself (`live.rs`), with the
//! signing and parsing ported from douyinLive (<https://github.com/jwwsjlm/douyinLive>,
//! MIT). When a douyinLive program path is set, it starts that program instead
//! and reads its JSON on `ws://127.0.0.1:<port>/ws/<room>`. Either way Avatar3D
//! stays connected whenever a room is set, independent of the Vidu session:
//! comments appear in the event log before going live and are handled like
//! any other audience event once live.

mod live;
pub mod page;
pub mod params;
mod proto;
#[cfg(test)]
mod recorded;
pub mod sign;

use crate::{
    audience::{self, Incoming},
    config,
    state::{App, lock},
};
use anyhow::{Context, Result, bail, ensure};
use futures_util::StreamExt;
use serde_json::Value;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};
use tokio::process::Child;
use tokio_tungstenite::{connect_async, tungstenite::Message};

/// A gift combo with no update for this long is treated as finished.
const COMBO_IDLE: Duration = Duration::from_secs(3);

#[derive(Clone, PartialEq)]
struct Target {
    room: String,
    port: u16,
    /// douyinLive to start; empty means Avatar3D connects by itself.
    program: String,
    /// The saved Douyin cookie, used by the built-in connection.
    cookie: Option<String>,
}
impl Target {
    /// The room to follow; None unless Douyin is the chosen platform.
    fn of(app: &App) -> Option<Self> {
        let c = app.config();
        if c.live_platform != "douyin" {
            return None;
        }
        Some(Self {
            room: room_id(&c.douyin_room)?,
            port: c.douyin_port,
            program: c.douyin_program.trim().to_owned(),
            cookie: config::key("douyin").ok().flatten(),
        })
    }
}

/// The room identifier from a bare ID or a live.douyin.com link, checked
/// with douyinLive's rule (`params::live_id`).
pub fn room_id(input: &str) -> Option<String> {
    let s = input.trim();
    let s = match s.find("live.douyin.com/") {
        Some(i) => &s[i + "live.douyin.com/".len()..],
        None => s,
    };
    params::live_id(s.split(['/', '?', '#']).next()?).map(str::to_owned)
}

fn status(app: &App, text: impl Into<String>) {
    lock(&app.view).douyin = text.into();
}

enum Outcome {
    Quit,
    /// The room, port or program changed in the settings.
    Changed,
    /// Could not reach douyinLive; retry with backoff.
    Failed(String),
    /// Was connected, then the connection dropped; retry soon.
    Lost(String),
    /// The room does not exist; retry rarely.
    Rejected(String),
    /// The room is not live; check again later.
    Offline(String),
}

/// Keeps one douyinLive connection open for the room in the settings.
pub async fn run(app: App) {
    let mut service: Option<(Target, Child)> = None;
    let mut backoff = 0;
    loop {
        let Some(target) = Target::of(&app) else {
            stop(&mut service).await;
            status(&app, "");
            backoff = 0;
            if pause(&app, None, Duration::from_secs(3600)).await {
                break;
            }
            continue;
        };
        if service
            .as_ref()
            .is_some_and(|(t, _)| t.port != target.port || t.program != target.program)
        {
            stop(&mut service).await;
        }
        let outcome = if target.program.is_empty() {
            stop(&mut service).await;
            live::listen(&app, &target).await
        } else {
            match start(&app, &target, &mut service).await {
                Ok(()) => listen(&app, &target).await,
                Err(e) => Outcome::Failed(e.to_string()),
            }
        };
        let wait = match outcome {
            Outcome::Quit => break,
            Outcome::Changed => {
                backoff = 0;
                continue;
            }
            Outcome::Failed(msg) => {
                backoff = (backoff + 3).min(30);
                status(&app, format!("{msg}。{backoff} 秒后重试"));
                backoff
            }
            Outcome::Lost(msg) => {
                backoff = 0;
                status(&app, format!("{msg}。3 秒后重连"));
                3
            }
            Outcome::Rejected(msg) => {
                status(&app, format!("{msg}。请检查直播间号"));
                60
            }
            Outcome::Offline(msg) => {
                backoff = 0;
                status(&app, msg);
                30
            }
        };
        if pause(&app, Some(&target), Duration::from_secs(wait)).await {
            break;
        }
    }
    stop(&mut service).await;
}

/// Sleeps up to `limit`, ending early when the settings change. True means quit.
async fn pause(app: &App, current: Option<&Target>, limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        tokio::select! {
            _ = app.quit.cancelled() => return true,
            _ = tokio::time::sleep(Duration::from_secs(1)) => (),
        }
        if Target::of(app).as_ref() != current {
            return false;
        }
    }
    false
}

async fn stop(service: &mut Option<(Target, Child)>) {
    if let Some((_, mut child)) = service.take() {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
}

async fn healthy(app: &App, port: u16) -> bool {
    app.http
        .get(format!("http://127.0.0.1:{port}/health"))
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .is_ok_and(|r| r.status().is_success())
}

/// Makes sure douyinLive answers on the port, starting the configured program if needed.
async fn start(app: &App, t: &Target, service: &mut Option<(Target, Child)>) -> Result<()> {
    if healthy(app, t.port).await {
        return Ok(());
    }
    ensure!(
        !t.program.is_empty(),
        "连不上本机 douyinLive（端口 {}）。请先运行 douyinLive，或在设置里填写程序路径",
        t.port
    );
    if let Some((_, child)) = service.as_mut()
        && child.try_wait()?.is_some()
    {
        *service = None;
    }
    if service.is_none() {
        let program = PathBuf::from(&t.program);
        ensure!(program.is_file(), "找不到 douyinLive 程序：{}", t.program);
        let log = fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(app.dir.join("douyin.log"))?;
        status(app, "正在启动 douyinLive");
        // douyinLive reads config.yaml from its own folder; the port flag overrides it.
        let child = tokio::process::Command::new(&program)
            .args(["--port", &t.port.to_string()])
            .current_dir(program.parent().unwrap_or(Path::new("/")))
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log)
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("无法启动 douyinLive：{}", t.program))?;
        *service = Some((t.clone(), child));
    }
    for _ in 0..40 {
        if healthy(app, t.port).await {
            return Ok(());
        }
        if let Some((_, child)) = service.as_mut()
            && let Some(code) = child.try_wait()?
        {
            *service = None;
            bail!("douyinLive 启动后退出了（{code}），详见数据目录中的 douyin.log");
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    bail!("douyinLive 20 秒内没有就绪，详见数据目录中的 douyin.log")
}

async fn listen(app: &App, t: &Target) -> Outcome {
    let url = format!("ws://127.0.0.1:{}/ws/{}", t.port, t.room);
    status(app, format!("正在连接直播间 {}", t.room));
    let mut ws = match tokio::time::timeout(Duration::from_secs(10), connect_async(&url)).await {
        Ok(Ok((ws, _))) => ws,
        Ok(Err(e)) => return Outcome::Failed(format!("连不上 douyinLive：{e}")),
        Err(_) => return Outcome::Failed("连接 douyinLive 超时".into()),
    };
    status(
        app,
        format!("已连上 douyinLive，等待直播间 {} 的状态", t.room),
    );
    let mut combos = Combos::default();
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    let outcome = loop {
        tokio::select! {
            _ = app.quit.cancelled() => break Outcome::Quit,
            _ = tick.tick() => {
                for gift in combos.due(Instant::now()) {
                    deliver_gift(app, gift);
                }
                if Target::of(app).as_ref() != Some(t) {
                    break Outcome::Changed;
                }
            }
            frame = ws.next() => match frame {
                None | Some(Ok(Message::Close(_))) => break Outcome::Lost("douyinLive 断开了连接".into()),
                Some(Err(e)) => break Outcome::Lost(format!("douyinLive 连接出错：{e}")),
                Some(Ok(Message::Text(s))) => {
                    let Ok(v) = serde_json::from_str::<Value>(&s) else { continue };
                    match parse(&v) {
                        Some(Event::Status { text, not_found }) => {
                            if not_found {
                                break Outcome::Rejected(text);
                            }
                            status(app, text);
                        }
                        Some(e) => handle(app, &mut combos, e),
                        None => (),
                    }
                }
                Some(Ok(_)) => (),
            }
        }
    };
    // Thank for combos still open when the connection ends.
    for gift in combos.drain() {
        deliver_gift(app, gift);
    }
    let _ = ws.close(None).await;
    outcome
}

fn deliver(app: &App, incoming: Incoming) {
    // Events receive() rejects, such as empty comments, are skipped.
    let _ = audience::receive(app, incoming, "douyin");
}

fn deliver_gift(app: &App, gift: Gift) {
    deliver(app, gift.incoming());
}

/// Passes comments and joins on, and merges gift combos before thanking.
fn handle(app: &App, combos: &mut Combos, event: Event) {
    match event {
        Event::Comment(incoming) => deliver(app, incoming),
        Event::Gift(gift) => {
            if let Some(done) = combos.feed(gift, Instant::now()) {
                deliver_gift(app, done);
            }
        }
        Event::Status { .. } => (),
    }
}

enum Event {
    Status { text: String, not_found: bool },
    Comment(Incoming),
    Gift(Gift),
}

struct Gift {
    /// Messages with the same key belong to one combo.
    key: String,
    id: String,
    end: bool,
    author: String,
    viewer: Option<String>,
    name: String,
    count: u32,
}
impl Gift {
    fn incoming(self) -> Incoming {
        Incoming {
            id: self.id,
            author: self.author,
            viewer_id: self.viewer,
            text: String::new(),
            gift: true,
            kind: Some("gift".into()),
            platform: Some("douyin".into()),
            gift_name: Some(self.name),
            gift_count: Some(self.count),
        }
    }
}

fn text(v: &Value) -> &str {
    v.as_str().unwrap_or("").trim()
}
/// protojson writes 64-bit integers as strings.
fn number(v: &Value) -> u64 {
    v.as_u64()
        .or_else(|| v.as_str()?.trim().parse().ok())
        .unwrap_or(0)
}
fn id(v: &Value) -> Option<String> {
    let s = match v {
        Value::String(s) => s.trim().to_owned(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    (!s.is_empty() && s != "0").then_some(s)
}

fn parse(v: &Value) -> Option<Event> {
    if v["type"] == "system" {
        let code = text(&v["code"]);
        let shown = [&v["status_text"], &v["message"], &v["code"]]
            .into_iter()
            .map(text)
            .find(|s| !s.is_empty())
            .unwrap_or("状态未知");
        let anchor = text(&v["live_name"]);
        let line = match code {
            "ROOM_ONLINE" if !anchor.is_empty() => format!("「{anchor}」正在直播，弹幕已接入"),
            "ROOM_ONLINE" => "直播间正在直播，弹幕已接入".into(),
            "ROOM_OFFLINE" | "ACCOUNT_OFFLINE_NO_ROOM" | "ROOM_ENDED" => {
                format!("{shown}，开播后自动接入")
            }
            _ => shown.to_owned(),
        };
        return Some(Event::Status {
            text: line,
            not_found: code == "ROOM_NOT_FOUND",
        });
    }
    let user = &v["user"];
    let viewer = [&user["id"], &user["idStr"], &user["secUid"]]
        .into_iter()
        .find_map(id);
    let msg_id = id(&v["common"]["msgId"]);
    let nickname = text(&user["nickname"]);
    match v["method"].as_str()? {
        "WebcastChatMessage" => events::chat(msg_id, nickname, viewer, text(&v["content"])),
        "WebcastMemberMessage" => events::join(msg_id, nickname, viewer, number(&v["action"])),
        "WebcastGiftMessage" => events::gift(events::GiftFields {
            msg_id,
            nickname,
            viewer,
            name: text(&v["gift"]["name"]),
            counts: ["repeatCount", "comboCount", "groupCount", "totalCount"]
                .map(|k| number(&v[k])),
            group: id(&v["groupId"]),
            end: number(&v["repeatEnd"]) == 1,
        }),
        _ => None,
    }
}

/// Audience events from Douyin message fields, shared by both connections.
mod events {
    use super::{Event, Gift};
    use crate::audience::Incoming;

    fn author(nickname: &str) -> String {
        let name = nickname.trim().chars().take(80).collect::<String>();
        if name.is_empty() {
            "神秘人".into()
        } else {
            name
        }
    }
    fn fresh() -> String {
        uuid::Uuid::new_v4().to_string()
    }
    fn id(msg_id: Option<String>) -> String {
        msg_id
            .filter(|s| !s.is_empty() && s != "0")
            .unwrap_or_else(fresh)
    }

    pub fn chat(
        msg_id: Option<String>,
        nickname: &str,
        viewer: Option<String>,
        content: &str,
    ) -> Option<Event> {
        let content = content.trim();
        if content.is_empty() {
            return None;
        }
        Some(Event::Comment(Incoming {
            id: id(msg_id),
            author: author(nickname),
            viewer_id: viewer,
            text: content.to_owned(),
            gift: false,
            kind: Some("chat".into()),
            platform: Some("douyin".into()),
            gift_name: None,
            gift_count: None,
        }))
    }

    /// Action 1 is entering the room.
    pub fn join(
        msg_id: Option<String>,
        nickname: &str,
        viewer: Option<String>,
        action: u64,
    ) -> Option<Event> {
        if action != 1 || nickname.trim().is_empty() {
            return None;
        }
        Some(Event::Comment(Incoming {
            id: id(msg_id),
            author: author(nickname),
            viewer_id: viewer,
            text: String::new(),
            gift: false,
            kind: Some("join".into()),
            platform: Some("douyin".into()),
            gift_name: None,
            gift_count: None,
        }))
    }

    pub struct GiftFields<'a> {
        pub msg_id: Option<String>,
        pub nickname: &'a str,
        pub viewer: Option<String>,
        pub name: &'a str,
        /// repeat, combo, group and total counts.
        pub counts: [u64; 4],
        pub group: Option<String>,
        pub end: bool,
    }

    pub fn gift(f: GiftFields) -> Option<Event> {
        let name = f.name.trim();
        if name.is_empty() {
            return None;
        }
        // Same count rule as the earlier Tiktok_Live reader: the largest count field.
        let count = f
            .counts
            .into_iter()
            .max()
            .unwrap_or(0)
            .clamp(1, u64::from(u32::MAX)) as u32;
        let author = author(f.nickname);
        let (key, gift_id) = match f.group.filter(|g| !g.is_empty() && g != "0") {
            Some(group) => (format!("g{group}"), format!("gift-g{group}")),
            None => (
                format!("{}|{name}", f.viewer.as_deref().unwrap_or(&author)),
                format!("gift-m{}", id(f.msg_id)),
            ),
        };
        Some(Event::Gift(Gift {
            key,
            id: gift_id,
            end: f.end,
            author,
            viewer: f.viewer,
            name: name.to_owned(),
            count,
        }))
    }
}

/// Merges Douyin gift combos so each combo is thanked once.
#[derive(Default)]
struct Combos {
    open: HashMap<String, (Gift, Instant)>,
}
impl Combos {
    /// Adds one gift message and returns the combo if this message ends it.
    fn feed(&mut self, gift: Gift, at: Instant) -> Option<Gift> {
        let end = gift.end;
        let key = gift.key.clone();
        match self.open.get_mut(&key) {
            Some((open, last)) => {
                open.count = open.count.max(gift.count);
                *last = at;
            }
            None => {
                self.open.insert(key.clone(), (gift, at));
            }
        }
        if end {
            return self.open.remove(&key).map(|(g, _)| g);
        }
        None
    }
    /// Combos that have been idle for COMBO_IDLE.
    fn due(&mut self, at: Instant) -> Vec<Gift> {
        self.open
            .extract_if(|_, (_, last)| at.duration_since(*last) >= COMBO_IDLE)
            .map(|(_, (g, _))| g)
            .collect()
    }
    fn drain(&mut self) -> Vec<Gift> {
        self.open.drain().map(|(_, (g, _))| g).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_room_from_id_or_link() {
        assert_eq!(room_id(" 516466932480 ").as_deref(), Some("516466932480"));
        assert_eq!(
            room_id("https://live.douyin.com/516466932480?enter_from=web").as_deref(),
            Some("516466932480")
        );
        assert_eq!(
            room_id("live.douyin.com/abc_1-x/").as_deref(),
            Some("abc_1-x")
        );
        assert!(room_id("a.b").is_none());
        assert!(room_id("").is_none());
        assert!(room_id("../ws").is_none());
        assert!(room_id(".").is_none());
        assert!(room_id("有中文").is_none());
    }

    #[test]
    fn parses_douyinlive_messages() {
        let chat = json!({"method":"WebcastChatMessage","common":{"msgId":"7312"},
            "user":{"id":"42","nickname":"小明"},"content":" 主播好 "});
        let Some(Event::Comment(c)) = parse(&chat) else {
            panic!("chat")
        };
        assert_eq!(
            (c.id.as_str(), c.author.as_str(), c.text.as_str()),
            ("7312", "小明", "主播好")
        );
        assert_eq!(c.viewer_id.as_deref(), Some("42"));
        assert_eq!(c.kind.as_deref(), Some("chat"));

        let join = json!({"method":"WebcastMemberMessage","action":"1","user":{"nickname":"小红"}});
        let Some(Event::Comment(j)) = parse(&join) else {
            panic!("join")
        };
        assert_eq!(j.kind.as_deref(), Some("join"));
        let left = json!({"method":"WebcastMemberMessage","action":"3","user":{"nickname":"小红"}});
        assert!(parse(&left).is_none());

        let gift = json!({"method":"WebcastGiftMessage","groupId":"99","repeatCount":"5",
            "comboCount":"3","repeatEnd":1,"gift":{"name":"小心心"},"user":{"nickname":"老王"}});
        let Some(Event::Gift(g)) = parse(&gift) else {
            panic!("gift")
        };
        assert_eq!(
            (g.key.as_str(), g.id.as_str(), g.count, g.end),
            ("g99", "gift-g99", 5, true)
        );

        let online = json!({"type":"system","code":"ROOM_ONLINE","live_name":"小柔"});
        let Some(Event::Status { text, not_found }) = parse(&online) else {
            panic!("status")
        };
        assert!(text.contains("小柔") && !not_found);
        let missing = json!({"type":"system","code":"ROOM_NOT_FOUND","status_text":"直播间不存在或房间号无效"});
        assert!(matches!(
            parse(&missing),
            Some(Event::Status {
                not_found: true,
                ..
            })
        ));
        assert!(parse(&json!({"method":"WebcastLikeMessage"})).is_none());
    }

    #[test]
    fn merges_gift_combos() {
        let gift = |count, end| Gift {
            key: "g1".into(),
            id: "gift-g1".into(),
            end,
            author: "老王".into(),
            viewer: None,
            name: "小心心".into(),
            count,
        };
        let t0 = Instant::now();
        let mut combos = Combos::default();
        assert!(combos.feed(gift(1, false), t0).is_none());
        assert!(combos.feed(gift(4, false), t0).is_none());
        assert_eq!(combos.feed(gift(2, true), t0).unwrap().count, 4);
        assert!(combos.open.is_empty());

        assert!(combos.feed(gift(7, false), t0).is_none());
        assert!(combos.due(t0 + Duration::from_secs(1)).is_empty());
        let idle = combos.due(t0 + COMBO_IDLE);
        assert_eq!(idle.len(), 1);
        assert_eq!(idle[0].count, 7);
    }
}
