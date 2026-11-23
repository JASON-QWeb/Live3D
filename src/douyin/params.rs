//! Query strings and small parsers for the Douyin web protocol, ported from
//! douyinLive (MIT, jwwsjlm): `websocket_params.go`, `heartbeat.go`,
//! `room_info.go`, `session_profile.go`, `http_context.go` and `live_id.go`.
//!
//! Depends only on std and `md-5` so the recording tools in
//! `scripts/douyin/` can compile it unchanged.

use super::sign::md5_hex;

pub const AID: &str = "6383";
const APP_NAME: &str = "douyin_web";
const VERSION_CODE: &str = "180800";
const SDK_VERSION: &str = "1.0.15";
const DEFAULT_WRDS_VERSION: &str = "7382620942951772256";
pub const DEFAULT_CURSOR: &str = "d-1_u-1_fh-7383731312643626035_t-1719159695790_r-1";
pub const DEFAULT_PUSH_URL: &str = "wss://webcast100-ws-web-lf.douyin.com/webcast/im/push/v2/";
const PUSH_PATH: &str = "/webcast/im/push/v2/";
/// need_persist_msg_count for the web profile.
const WEB_PERSIST_MSG_COUNT: &str = "15";

/// Go's url.QueryEscape: unreserved bytes kept, space as "+", the rest as %XX.
pub fn query_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(char::from(b))
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// QueryEscape with spaces as %20 (`queryEscapeValue`).
pub fn query_escape_value(value: &str) -> String {
    query_escape(value).replace('+', "%20")
}

/// Only spaces escaped (`websocketQueryValue`).
fn websocket_query_value(value: &str) -> String {
    value.replace(' ', "%20")
}

/// The text after "Mozilla/", or the whole UA.
pub fn browser_version(user_agent: &str) -> &str {
    user_agent
        .split_once("Mozilla/")
        .map_or(user_agent, |(_, rest)| rest)
}

/// The full Chrome version in a UA, else `browser_version`.
pub fn chrome_version(user_agent: &str) -> &str {
    if let Some(idx) = user_agent.find("Chrome/") {
        let version = &user_agent[idx + "Chrome/".len()..];
        let version = version.split(' ').next().unwrap_or(version);
        if !version.is_empty() {
            return version;
        }
    }
    browser_version(user_agent)
}

/// MD5 of the canonical signing input (`XMSStub`).
pub fn websocket_stub(room_id: &str, user_unique_id: &str) -> String {
    let fields = [
        ("live_id", "1"),
        ("aid", AID),
        ("version_code", VERSION_CODE),
        ("webcast_sdk_version", SDK_VERSION),
        ("room_id", room_id),
        ("sub_room_id", ""),
        ("sub_channel_id", ""),
        ("did_rule", "3"),
        ("user_unique_id", user_unique_id),
        ("device_platform", "web"),
        ("device_type", ""),
        ("ac", ""),
        ("identity", "audience"),
    ];
    let joined = fields
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(",");
    md5_hex(joined.as_bytes())
}

/// Screen size in the query strings.
#[derive(Clone, Copy)]
pub struct Screen {
    pub width: i64,
    pub height: i64,
}
impl Screen {
    /// 1920 × 1080 in place of a size that is not positive (`browserFingerprint.screenSize`).
    pub fn normalized(self) -> Self {
        Self {
            width: if self.width > 0 { self.width } else { 1920 },
            height: if self.height > 0 { self.height } else { 1080 },
        }
    }
}

/// The WebSocket query string for the web profile (`websocketURLParams.QueryString`).
pub fn websocket_query(
    room_id: &str,
    push_id: &str,
    user_agent: &str,
    cursor: &str,
    internal_ext: &str,
    signature: &str,
    screen: Screen,
) -> String {
    [
        format!("app_name={APP_NAME}"),
        format!("version_code={VERSION_CODE}"),
        format!("webcast_sdk_version={SDK_VERSION}"),
        format!("update_version_code={SDK_VERSION}"),
        "compress=gzip".into(),
        "device_platform=web".into(),
        "cookie_enabled=true".into(),
        format!("screen_width={}", screen.width),
        format!("screen_height={}", screen.height),
        "browser_language=zh-CN".into(),
        "browser_platform=Win32".into(),
        "browser_name=Mozilla".into(),
        format!(
            "browser_version={}",
            websocket_query_value(browser_version(user_agent))
        ),
        "browser_online=true".into(),
        "tz_name=Asia/Shanghai".into(),
        format!("cursor={cursor}"),
        format!("internal_ext={internal_ext}"),
        "host=https://live.douyin.com".into(),
        format!("aid={AID}"),
        "live_id=1".into(),
        "did_rule=3".into(),
        "endpoint=live_pc".into(),
        "support_wrds=1".into(),
        format!("user_unique_id={push_id}"),
        "im_path=/webcast/im/fetch/".into(),
        "identity=audience".into(),
        format!("need_persist_msg_count={WEB_PERSIST_MSG_COUNT}"),
        "insert_task_id=".into(),
        "live_reason=".into(),
        format!("room_id={room_id}"),
        "heartbeatDuration=0".into(),
        format!("signature={}", websocket_query_value(signature)),
    ]
    .join("&")
}

/// The im/fetch query string (`initialIMFetchParams.QueryString`).
pub fn im_fetch_query(
    room_id: &str,
    push_id: &str,
    user_agent: &str,
    ms_token: &str,
    screen: Screen,
) -> String {
    [
        "resp_content_type=protobuf".into(),
        "did_rule=3".into(),
        "device_id=".into(),
        format!("app_name={APP_NAME}"),
        "endpoint=live_pc".into(),
        "support_wrds=1".into(),
        format!("user_unique_id={}", query_escape(push_id)),
        "identity=audience".into(),
        "need_persist_msg_count=15".into(),
        "insert_task_id=".into(),
        "live_reason=".into(),
        format!("room_id={}", query_escape(room_id)),
        format!("version_code={VERSION_CODE}"),
        "last_rtt=0".into(),
        "live_id=1".into(),
        format!("aid={AID}"),
        "fetch_rule=1".into(),
        "cursor=".into(),
        "internal_ext=".into(),
        "device_platform=web".into(),
        "cookie_enabled=true".into(),
        format!("screen_width={}", screen.width),
        format!("screen_height={}", screen.height),
        "browser_language=zh-CN".into(),
        "browser_platform=Win32".into(),
        "browser_name=Mozilla".into(),
        format!(
            "browser_version={}",
            query_escape(browser_version(user_agent))
        ),
        "browser_online=true".into(),
        "tz_name=Asia/Shanghai".into(),
        format!("msToken={}", query_escape(ms_token)),
    ]
    .join("&")
}

/// The web/enter query string (`buildRoomEnterParams`). `room_id` is the
/// current room only while the room is known to be live, else empty.
pub fn room_enter_query(
    live_id: &str,
    room_id: &str,
    user_agent: &str,
    ms_token: &str,
    screen: Screen,
) -> String {
    let screen = screen.normalized();
    [
        format!("aid={AID}"),
        format!("app_name={APP_NAME}"),
        "live_id=1".into(),
        "device_platform=web".into(),
        "language=zh-CN".into(),
        "enter_from=link_share".into(),
        "cookie_enabled=true".into(),
        format!("screen_width={}", screen.width),
        format!("screen_height={}", screen.height),
        "browser_language=zh-CN".into(),
        "browser_platform=Win32".into(),
        "browser_name=Chrome".into(),
        format!(
            "browser_version={}",
            query_escape_value(chrome_version(user_agent))
        ),
        "os_name=Windows".into(),
        "os_version=10".into(),
        format!("web_rid={}", query_escape_value(live_id)),
        format!("room_id_str={}", query_escape_value(room_id)),
        "enter_source=".into(),
        "is_need_double_stream=false".into(),
        "insert_task_id=".into(),
        "live_reason=".into(),
        format!("msToken={}", query_escape_value(ms_token)),
    ]
    .join("&")
}

/// The internal_ext sent before the server gives one (`defaultInternalExt`).
pub fn default_internal_ext(room_id: &str, user_unique_id: &str, now_ms: i64) -> String {
    format!(
        "internal_src:dim|wss_push_room_id:{room_id}|wss_push_did:{user_unique_id}|first_req_ms:{now_ms}|fetch_time:{now_ms}|seq:1|wss_info:0-{now_ms}-0-0|wrds_v:{DEFAULT_WRDS_VERSION}"
    )
}

/// A push server from im/fetch as a WebSocket URL ending in the push path
/// (`normalizeWebsocketPushURL`). Empty input gives an empty string.
pub fn push_url(candidate: &str) -> String {
    let mut candidate = candidate.trim();
    if candidate.is_empty() {
        return String::new();
    }
    if let Some(idx) = candidate.find([',', ';']) {
        candidate = candidate[..idx].trim();
    }
    let candidate = candidate.trim_end_matches('/');
    let url = if candidate.starts_with("wss://") || candidate.starts_with("ws://") {
        candidate.to_owned()
    } else if let Some(rest) = candidate.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = candidate.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        format!("wss://{candidate}")
    };
    if url.contains(PUSH_PATH.trim_end_matches('/')) {
        url + "/"
    } else {
        url + PUSH_PATH
    }
}

/// Seconds in a `Handshake-Options: ping-interval=15;` header, or 0.
pub fn ping_interval(header: &str) -> i64 {
    for option in header.trim().split(';') {
        let Some((key, value)) = option.trim().split_once('=') else {
            continue;
        };
        if !key.trim().eq_ignore_ascii_case("ping-interval") {
            continue;
        }
        return match value.trim().parse::<i64>() {
            Ok(seconds) if seconds > 0 => seconds,
            _ => 0,
        };
    }
    0
}

/// The room identifier, trimmed, if it is 1 to 128 ASCII letters, digits, `_` or `-`
/// (`ValidateLiveID`).
pub fn live_id(value: &str) -> Option<&str> {
    let value = value.trim();
    let valid = !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    valid.then_some(value)
}
