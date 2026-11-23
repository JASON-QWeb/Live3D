//! A Douyin live-room connection made by Vorela itself, ported from
//! douyinLive (MIT, jwwsjlm) with its web protocol profile:
//!
//! 1. Get an anonymous `ttwid` cookie, unless a Douyin cookie is saved
//!    (`cookie_context.go`).
//! 2. Read room identity and live status from the live page, then from the
//!    signed web/enter API (`room_info.go`).
//! 3. Fetch the starting cursor and push server from im/fetch, signed with
//!    a_bogus (`websocket_connection.go`).
//! 4. Open the signed WebSocket, send heartbeats, acknowledge frames and
//!    decode messages (`heartbeat.go`, `message_decode.go`).
//!
//! Reconnecting is left to the caller, which starts over from step 2.

use super::{
    Event, Outcome, Target, deliver_gift, events, page,
    params::{self, Screen},
    proto::{ChatMessage, ControlMessage, GiftMessage, MemberMessage, PushFrame, Response},
    sign::{WebsocketSigner, a_bogus_at},
    status,
};
use crate::state::App;
use futures_util::{SinkExt, StreamExt};
use prost::Message as _;
use std::{
    collections::BTreeMap,
    io::Read,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{self, Message, client::IntoClientRequest, http::HeaderValue},
};

const HTTP_TIMEOUT: Duration = Duration::from_secs(15);
const READ_TIMEOUT: Duration = Duration::from_secs(70);
const MIN_HEARTBEAT: Duration = Duration::from_secs(10);
const MAX_GZIP: u64 = 32 << 20;
const PAGE_LIMIT: usize = 8 << 20;
/// ControlMessage action for "the live has ended".
const LIVE_END: u64 = 3;

/// The web profile's user agents. They name Chrome 133 because the original
/// pairs them with a Chrome 133 TLS fingerprint.
const USER_AGENTS: [&str; 2] = [
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/133.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Windows NT 10.0; WOW64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/133.0.0.0 Safari/537.36",
];
const CLIENT_HINTS: [(&str, &str); 3] = [
    (
        "sec-ch-ua",
        r#""Not;A=Brand";v="8", "Chromium";v="133", "Google Chrome";v="133""#,
    ),
    ("sec-ch-ua-mobile", "?0"),
    ("sec-ch-ua-platform", r#""Windows""#),
];
/// Screen sizes of the original's browser fingerprint presets.
const SCREENS: [(i64, i64); 5] = [
    (1920, 1080),
    (1920, 1080),
    (1536, 864),
    (1366, 768),
    (2560, 1440),
];

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut buf = [0u8; N];
    // INFALLIBLE: the OS random source does not fail on macOS; fall back to
    // the clock so a failure cannot stop the connection.
    if getrandom::fill(&mut buf).is_err() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        for (i, b) in buf.iter_mut().enumerate() {
            *b = (nanos >> (8 * (i % 16))) as u8;
        }
    }
    buf
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// A random 172-character msToken (`GenerateMsToken`).
fn ms_token() -> String {
    const CHARSET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    random_bytes::<172>()
        .iter()
        .map(|b| char::from(CHARSET[usize::from(b & 63)]))
        .collect()
}

/// `name=value` pairs of a cookie header, trimmed (`ParseCookies`).
fn parse_cookie(cookie: &str) -> Vec<(String, String)> {
    cookie
        .split(';')
        .filter_map(|pair| {
            let (name, value) = pair.trim().split_once('=')?;
            Some((name.trim().to_owned(), value.trim().to_owned()))
        })
        .collect()
}

/// Why the connection could not be set up.
enum Setup {
    /// The room exists and is not live now.
    Offline(String),
    /// The room does not exist.
    NotFound(String),
    Failed(String),
}

struct Session {
    http: reqwest::Client,
    live_id: String,
    user_agent: &'static str,
    screen: Screen,
    /// The saved Douyin cookie, which replaces the anonymous ttwid.
    configured: Vec<(String, String)>,
    ttwid: String,
    extra_cookies: BTreeMap<String, String>,
    generated_token: String,
    info: page::RoomInfo,
    /// Some(true) once confirmed live, Some(false) once confirmed offline.
    live: Option<bool>,
    cursor: String,
    internal_ext: String,
    push_url: String,
    heartbeat: Duration,
    signer: WebsocketSigner,
}

impl Session {
    fn new(target: &Target) -> Result<Self, String> {
        let [ua, screen] = random_bytes::<2>();
        let (width, height) = SCREENS[usize::from(screen) % SCREENS.len()];
        let user_agent = USER_AGENTS[usize::from(ua) % USER_AGENTS.len()];
        let http = reqwest::Client::builder()
            .user_agent(user_agent)
            .timeout(HTTP_TIMEOUT)
            .build()
            .map_err(|e| format!("无法创建网络客户端：{e}"))?;
        Ok(Self {
            http,
            live_id: target.room.clone(),
            user_agent,
            screen: Screen { width, height },
            configured: parse_cookie(target.cookie.as_deref().unwrap_or("")),
            ttwid: String::new(),
            extra_cookies: BTreeMap::new(),
            generated_token: String::new(),
            info: page::RoomInfo::default(),
            live: None,
            cursor: String::new(),
            internal_ext: String::new(),
            push_url: String::new(),
            heartbeat: MIN_HEARTBEAT,
            signer: WebsocketSigner::default(),
        })
    }

    /// The Cookie header (`getCookieString`).
    fn cookie(&self) -> String {
        if !self.configured.is_empty() {
            return self
                .configured
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("; ");
        }
        std::iter::once(format!("ttwid={}", self.ttwid))
            .chain(self.extra_cookies.iter().map(|(k, v)| format!("{k}={v}")))
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// msToken from the saved cookie, else one generated per session.
    fn ms_token(&mut self) -> String {
        if let Some((_, v)) = self.configured.iter().find(|(k, _)| k == "msToken")
            && !v.trim().is_empty()
        {
            return v.trim().to_owned();
        }
        if self.generated_token.is_empty() {
            self.generated_token = ms_token();
        }
        self.generated_token.clone()
    }

    fn get(&self, url: &str, accept: &str, referer: &str) -> reqwest::RequestBuilder {
        let mut req = self
            .http
            .get(url)
            .header("Accept", accept)
            .header("Accept-Encoding", "identity")
            .header("Accept-Language", "zh-CN,zh;q=0.9")
            .header("Cookie", self.cookie())
            .header("Referer", referer);
        for (k, v) in CLIENT_HINTS {
            req = req.header(k, v);
        }
        req
    }

    /// Keeps the ttwid and other cookies a response sets (`storeTTWIDCookies`).
    fn store_cookies(&mut self, resp: &reqwest::Response) -> bool {
        let mut cookies = BTreeMap::new();
        for header in resp.headers().get_all(reqwest::header::SET_COOKIE) {
            let Ok(text) = header.to_str() else { continue };
            let pair = text.split(';').next().unwrap_or("");
            if let Some((name, value)) = pair.split_once('=') {
                cookies.insert(name.trim().to_owned(), value.trim().to_owned());
            }
        }
        match cookies.remove("ttwid") {
            Some(ttwid) if !ttwid.trim().is_empty() => {
                self.ttwid = ttwid;
                self.extra_cookies = cookies;
                true
            }
            _ => false,
        }
    }

    /// An anonymous ttwid from the light registration API, else from the
    /// live homepage (`fetchTTWID`).
    async fn fetch_ttwid(&mut self) -> Result<(), String> {
        let registered = self
            .http
            .post("https://ttwid.bytedance.com/ttwid/union/register/")
            .header("Content-Type", "application/json")
            .body(r#"{"region":"cn","aid":1768,"needFid":false,"service":"www.ixigua.com","migrate_info":{"ticket":"","source":"node"},"cbUrlProtocol":"https","union":true}"#)
            .timeout(Duration::from_secs(2))
            .send()
            .await;
        if let Ok(resp) = registered
            && resp.status().is_success()
            && self.store_cookies(&resp)
        {
            return Ok(());
        }
        let resp = self
            .http
            .get("https://live.douyin.com/")
            .send()
            .await
            .map_err(|e| format!("获取 ttwid 失败：{}", e.without_url()))?;
        if !resp.status().is_success() {
            return Err(format!(
                "获取 ttwid 失败（HTTP {}）",
                resp.status().as_u16()
            ));
        }
        if self.store_cookies(&resp) {
            Ok(())
        } else {
            Err("抖音没有返回 ttwid".into())
        }
    }

    async fn body(resp: reqwest::Response, limit: usize) -> Result<Vec<u8>, String> {
        crate::audio::limited(resp, limit)
            .await
            .map(|b| b.to_vec())
            .map_err(|e| e.to_string())
    }

    /// Reads the live page. Err(Some(status)) when the page has no room or
    /// account on it, Err(None) when the request failed
    /// (`fetchLivePageStateWithContext`).
    async fn fetch_page(&mut self) -> Result<(), Option<u16>> {
        self.live = None;
        let resp = self
            .get(
                &format!("https://live.douyin.com/{}", self.live_id),
                "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8",
                "https://live.douyin.com/",
            )
            .send()
            .await
            .map_err(|_| None)?;
        let code = resp.status().as_u16();
        let body = Self::body(resp, PAGE_LIMIT).await.map_err(|_| None)?;
        let state = page::live_page(&String::from_utf8_lossy(&body));
        if !state.known() {
            return Err(Some(code));
        }
        let info = &mut self.info;
        if state.info.anchor_only {
            info.room_id.clear();
            info.anchor_only = true;
        } else if !state.info.room_id.is_empty() {
            info.room_id = state.info.room_id.clone();
            info.anchor_only = false;
        }
        for (field, value) in [
            (&mut info.push_id, &state.info.push_id),
            (&mut info.live_name, &state.info.live_name),
            (&mut info.title, &state.info.title),
            (&mut info.avatar, &state.info.avatar),
        ] {
            if !value.is_empty() {
                *field = value.clone();
            }
        }
        if state.status_known {
            self.live = Some(state.live);
        }
        Ok(())
    }

    /// One signed web/enter request (`doRequest`). Ok(None) is an empty body.
    async fn room_enter(&mut self) -> Result<Option<String>, String> {
        let room = if self.live == Some(true) {
            self.info.room_id.clone()
        } else {
            String::new()
        };
        let token = self.ms_token();
        let query =
            params::room_enter_query(&self.live_id, &room, self.user_agent, &token, self.screen);
        let url = format!(
            "https://live.douyin.com/webcast/room/web/enter/?{query}&a_bogus={}",
            params::query_escape(&a_bogus_at(&query, self.user_agent, now_ms()))
        );
        let resp = self
            .get(
                &url,
                "application/json, text/plain, */*",
                &format!("https://live.douyin.com/{}", self.live_id),
            )
            .send()
            .await
            .map_err(|e| format!("请求直播间信息失败：{}", e.without_url()))?;
        if !resp.status().is_success() {
            return Err(format!(
                "直播间信息接口响应异常（HTTP {}）",
                resp.status().as_u16()
            ));
        }
        let body = String::from_utf8_lossy(&Self::body(resp, PAGE_LIMIT).await?).into_owned();
        if body.is_empty() {
            return Ok(None);
        }
        let v = serde_json::from_str::<serde_json::Value>(&body)
            .map_err(|_| "直播间信息接口未返回有效 JSON".to_owned())?;
        match v["status_code"].as_f64() {
            Some(0.0) => Ok(Some(body)),
            Some(code) => Err(format!("直播间信息接口返回异常 status_code={code}")),
            None => Err("直播间信息接口未返回有效 status_code".into()),
        }
    }

    /// Room identity and live status from the page and web/enter
    /// (`prepareWebSocketContextLocked` up to the signer).
    async fn identify(&mut self) -> Result<(), Setup> {
        if self.configured.is_empty() && self.ttwid.trim().is_empty() {
            self.fetch_ttwid().await.map_err(Setup::Failed)?;
        }
        let page = self.fetch_page().await;
        // A 200 page with no room state is a verification page: retrying web/enter does not help.
        let verification_page = matches!(page, Err(Some(code)) if code != 404 && code != 410);
        let definitive_missing = matches!(page, Err(Some(404 | 410)));
        let attempts = if verification_page { 1 } else { 3 };
        let mut result = Ok(None);
        for attempt in 0..attempts {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            result = self.room_enter().await;
            let empty = matches!(result, Ok(None));
            let can_fallback = self.live == Some(true)
                && !self.info.room_id.is_empty()
                && !self.info.push_id.is_empty();
            if !empty || self.live == Some(false) || can_fallback {
                break;
            }
        }
        match result.map_err(Setup::Failed)? {
            Some(body) => {
                let info = page::room_enter(&body).map_err(|e| Setup::Failed(e.into()))?;
                if let Some(live) = page::room_enter_live(&body) {
                    self.live = Some(live);
                }
                let own = &mut self.info;
                if !info.room_id.is_empty() {
                    own.room_id = info.room_id;
                    own.anchor_only = false;
                }
                if own.push_id.is_empty() {
                    own.push_id = info.push_id;
                }
                for (field, value) in [
                    (&mut own.live_name, info.live_name),
                    (&mut own.title, info.title),
                    (&mut own.avatar, info.avatar),
                ] {
                    if !value.is_empty() {
                        *field = value;
                    }
                }
            }
            // An empty web/enter: the original falls back to the page while the page says live.
            None if self.live == Some(true)
                && !self.info.room_id.is_empty()
                && !self.info.push_id.is_empty() => {}
            None if self.live != Some(false) => {
                let nothing = self.info.room_id.is_empty()
                    && self.info.live_name.trim().is_empty()
                    && self.info.title.trim().is_empty()
                    && !self.info.anchor_only;
                if nothing && definitive_missing {
                    return Err(Setup::NotFound("直播间不存在或房间号无效".into()));
                }
                if verification_page {
                    return Err(Setup::Failed(
                        "抖音返回了验证页，可能是匿名访问受限。可以在「连接凭据」保存抖音 Cookie，或换个网络后再试".into(),
                    ));
                }
                return Err(Setup::Failed("直播间信息接口返回空响应".into()));
            }
            None => {}
        }
        match self.live {
            Some(true) => Ok(()),
            Some(false) => Err(Setup::Offline("直播间未开播，开播后自动接入".into())),
            None => Err(Setup::Failed("直播状态暂时无法确认".into())),
        }
    }

    /// Keeps the cursor, internal_ext, push server and heartbeat a response
    /// gives (`applyWebsocketResponseState`).
    fn apply(&mut self, r: &Response) {
        if !r.cursor.is_empty() {
            self.cursor = r.cursor.clone();
        }
        if !r.internal_ext.is_empty() {
            self.internal_ext = r.internal_ext.clone();
        }
        if let Some(url) = [&r.push_server_v2, &r.push_server, &r.proxy_server]
            .into_iter()
            .map(|c| params::push_url(c))
            .find(|u| !u.is_empty())
        {
            self.push_url = url;
        }
        if r.heartbeat_duration > 0 {
            self.heartbeat = Duration::from_secs(r.heartbeat_duration);
        }
    }

    /// The starting IM state; failures fall back to defaults (`fetchInitialIMState`).
    async fn fetch_im(&mut self) {
        let token = self.ms_token();
        let query = params::im_fetch_query(
            &self.info.room_id,
            &self.info.push_id,
            self.user_agent,
            &token,
            self.screen,
        );
        let url = format!(
            "https://live.douyin.com/webcast/im/fetch/?{query}&a_bogus={}",
            params::query_escape(&a_bogus_at(&query, self.user_agent, now_ms()))
        );
        let resp = self
            .get(
                &url,
                "*/*",
                &format!("https://live.douyin.com/{}", self.live_id),
            )
            .header(
                "Content-Type",
                "application/x-www-form-urlencoded; charset=UTF-8",
            )
            .send()
            .await;
        let Ok(resp) = resp else { return };
        let Ok(body) = Self::body(resp, 4 << 20).await else {
            return;
        };
        if let Ok(r) = Response::decode(body.as_slice())
            && [&r.push_server_v2, &r.push_server, &r.proxy_server]
                .iter()
                .any(|c| !params::push_url(c).is_empty())
        {
            self.apply(&r);
        }
    }

    /// The signed WebSocket URL (`buildWebsocketURL`).
    fn websocket_url(&mut self) -> Result<String, String> {
        let (room, push) = (&self.info.room_id, &self.info.push_id);
        let stub = params::websocket_stub(room, push);
        let signature = self.signer.sign(&stub, || random_bytes::<1>()[0])?;
        let cursor = if self.cursor.is_empty() {
            params::DEFAULT_CURSOR
        } else {
            &self.cursor
        };
        let ext = if self.internal_ext.is_empty() {
            params::default_internal_ext(room, push, now_ms())
        } else {
            self.internal_ext.clone()
        };
        let base = if self.push_url.is_empty() {
            params::DEFAULT_PUSH_URL
        } else {
            &self.push_url
        };
        let query = params::websocket_query(
            room,
            push,
            self.user_agent,
            cursor,
            &ext,
            &signature,
            self.screen,
        );
        Ok(format!("{base}?{query}"))
    }
}

/// A decoded webcast message as an audience event.
fn event(method: &str, payload: &[u8]) -> Option<Event> {
    match method {
        "WebcastChatMessage" => {
            let m = ChatMessage::decode(payload).ok()?;
            let user = m.user.unwrap_or_default();
            events::chat(
                m.common.map(|c| c.msg_id.to_string()),
                &user.nickname,
                user.viewer(),
                &m.content,
            )
        }
        "WebcastMemberMessage" => {
            let m = MemberMessage::decode(payload).ok()?;
            let user = m.user.unwrap_or_default();
            events::join(
                m.common.map(|c| c.msg_id.to_string()),
                &user.nickname,
                user.viewer(),
                m.action,
            )
        }
        "WebcastGiftMessage" => {
            let m = GiftMessage::decode(payload).ok()?;
            let user = m.user.unwrap_or_default();
            events::gift(events::GiftFields {
                msg_id: m.common.map(|c| c.msg_id.to_string()),
                nickname: &user.nickname,
                viewer: user.viewer(),
                name: &m.gift.unwrap_or_default().name,
                counts: [m.repeat_count, m.combo_count, m.group_count, m.total_count],
                group: (m.group_id != 0).then(|| m.group_id.to_string()),
                end: m.repeat_end == 1,
            })
        }
        _ => None,
    }
}

/// What one push frame asks for: an acknowledgement, events, or the live's end.
#[derive(Default)]
struct Decoded {
    ack: Option<Vec<u8>>,
    events: Vec<Event>,
    ended: bool,
}

/// Decodes a "msg" frame (`decodeResponse`, `decodeGzipResponse`).
fn decode(session: &mut Session, frame: &PushFrame) -> Result<Decoded, String> {
    let data = if frame.header("compress_type") == "gzip" {
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(frame.payload.as_slice())
            .take(MAX_GZIP + 1)
            .read_to_end(&mut out)
            .map_err(|e| format!("解压失败：{e}"))?;
        if out.len() as u64 > MAX_GZIP {
            return Err("消息过大".into());
        }
        out
    } else {
        frame.payload.clone()
    };
    let response = Response::decode(data.as_slice()).map_err(|e| format!("解析失败：{e}"))?;
    session.apply(&response);
    // The client echoes the frame's im-internal_ext header, not Response.internal_ext.
    let mut ack_ext = frame.header("im-internal_ext").to_owned();
    if ack_ext.is_empty() && response.need_ack {
        ack_ext = response.internal_ext.clone();
    }
    let mut out = Decoded {
        ack: (!ack_ext.is_empty()).then(|| PushFrame::ack(frame.log_id, &ack_ext)),
        ..Default::default()
    };
    for msg in &response.messages {
        if msg.method == "WebcastControlMessage" {
            if ControlMessage::decode(msg.payload.as_slice()).is_ok_and(|c| c.action == LIVE_END) {
                out.ended = true;
                break;
            }
            continue;
        }
        out.events.extend(event(&msg.method, &msg.payload));
    }
    Ok(out)
}

/// Follows the room until it ends, the connection drops or the settings change.
pub(super) async fn listen(app: &App, target: &Target) -> Outcome {
    let mut session = match Session::new(target) {
        Ok(s) => s,
        Err(e) => return Outcome::Failed(e),
    };
    status(app, format!("正在连接抖音直播间 {}", target.room));
    let setup = async {
        match session.identify().await {
            Err(e) => Err(e),
            Ok(()) => {
                session.fetch_im().await;
                Ok(())
            }
        }
    };
    let setup = tokio::select! {
        _ = app.quit.cancelled() => return Outcome::Quit,
        r = setup => r,
    };
    match setup {
        Ok(()) => (),
        Err(Setup::Offline(msg)) => return Outcome::Offline(msg),
        Err(Setup::NotFound(msg)) => return Outcome::Rejected(msg),
        Err(Setup::Failed(msg)) => return Outcome::Failed(msg),
    }
    let url = match session.websocket_url() {
        Ok(url) => url,
        Err(e) => return Outcome::Failed(e),
    };
    let mut request = match url.as_str().into_client_request() {
        Ok(r) => r,
        Err(e) => return Outcome::Failed(format!("WebSocket 地址无效：{e}")),
    };
    let referer = format!("https://live.douyin.com/{}", session.live_id);
    let cookie = session.cookie();
    let headers = [
        ("User-Agent", session.user_agent),
        ("Origin", "https://live.douyin.com"),
        ("Referer", referer.as_str()),
        ("Cookie", cookie.as_str()),
    ];
    for (k, v) in headers.into_iter().chain(CLIENT_HINTS) {
        if let Ok(value) = HeaderValue::from_str(v) {
            request.headers_mut().insert(k, value);
        }
    }
    let connected = tokio::time::timeout(Duration::from_secs(10), connect_async(request)).await;
    let (ws, response) = match connected {
        Ok(Ok(ok)) => ok,
        Ok(Err(tungstenite::Error::Http(resp))) => {
            return Outcome::Failed(format!(
                "抖音拒绝了弹幕连接（HTTP {}）",
                resp.status().as_u16()
            ));
        }
        Ok(Err(e)) => return Outcome::Failed(format!("弹幕连接失败：{e}")),
        Err(_) => return Outcome::Failed("弹幕连接超时".into()),
    };
    let ping = response
        .headers()
        .get("Handshake-Options")
        .and_then(|v| v.to_str().ok())
        .map_or(0, params::ping_interval);
    if ping > 0 {
        session.heartbeat = Duration::from_secs(ping as u64);
    }
    let name = session.info.live_name.trim().to_owned();
    status(
        app,
        if name.is_empty() {
            "直播间正在直播，弹幕已接入".to_owned()
        } else {
            format!("「{name}」正在直播，弹幕已接入")
        },
    );

    receive_frames(app, target, session, ws).await
}

async fn receive_frames(
    app: &App,
    target: &Target,
    mut session: Session,
    ws: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> Outcome {
    let (mut sink, mut stream) = ws.split();
    let mut combos = super::Combos::default();
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    let mut next_heartbeat = Instant::now();
    let mut last_read = Instant::now();
    let outcome = loop {
        tokio::select! {
            _ = app.quit.cancelled() => break Outcome::Quit,
            _ = tick.tick() => {
                for gift in combos.due(Instant::now()) {
                    deliver_gift(app, gift);
                }
                if Target::of(app).as_ref() != Some(target) {
                    break Outcome::Changed;
                }
                if last_read.elapsed() > READ_TIMEOUT {
                    break Outcome::Lost("弹幕连接 70 秒没有数据".into());
                }
                if Instant::now() >= next_heartbeat {
                    if let Err(e) = sink.send(Message::binary(PushFrame::heartbeat())).await {
                        break Outcome::Lost(format!("发送心跳失败：{e}"));
                    }
                    next_heartbeat = Instant::now() + session.heartbeat.max(MIN_HEARTBEAT);
                }
            }
            frame = stream.next() => {
                last_read = Instant::now();
                let data = match frame {
                    None => break Outcome::Lost("抖音断开了弹幕连接".into()),
                    Some(Err(e)) => break Outcome::Lost(format!("弹幕连接出错：{e}")),
                    Some(Ok(Message::Close(c))) => {
                        let why = c.map(|c| format!("（{}）", u16::from(c.code))).unwrap_or_default();
                        break Outcome::Lost(format!("抖音关闭了弹幕连接{why}"));
                    }
                    Some(Ok(Message::Binary(data))) if !data.is_empty() => data,
                    Some(Ok(_)) => continue,
                };
                let Ok(frame) = PushFrame::decode(data.as_ref()) else { continue };
                if frame.payload_type != "msg" {
                    continue;
                }
                let Ok(decoded) = decode(&mut session, &frame) else { continue };
                if let Some(ack) = decoded.ack
                    && let Err(e) = sink.send(Message::binary(ack)).await
                {
                    break Outcome::Lost(format!("发送确认失败：{e}"));
                }
                for e in decoded.events {
                    super::handle(app, &mut combos, e);
                }
                if decoded.ended {
                    break Outcome::Offline("直播已结束，开播后自动接入".into());
                }
            }
        }
    };
    for gift in combos.drain() {
        deliver_gift(app, gift);
    }
    let _ = sink.close().await;
    outcome
}

#[cfg(test)]
mod tests;
