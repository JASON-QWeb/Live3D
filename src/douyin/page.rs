//! Room identity and live status from the Douyin live page and the web/enter
//! response, ported from douyinLive's `room_info.go` (MIT, jwwsjlm).
//!
//! The original reads JSON with gjson paths. `path` and `text` reproduce the
//! parts of gjson it relies on: dotted keys, numeric array indexes, and
//! numbers read back as their digits. Depends only on std and `serde_json`.

use serde_json::Value;

/// What the original keeps about a room.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RoomInfo {
    pub room_id: String,
    /// The user_unique_id the WebSocket connection signs with.
    pub push_id: String,
    pub live_name: String,
    pub title: String,
    pub avatar: String,
    /// The account exists but has no room object right now.
    pub anchor_only: bool,
}

/// What the live page says (`livePageStateSnapshot`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PageState {
    pub info: RoomInfo,
    pub user_unique_id: String,
    pub has_anchor: bool,
    pub live: bool,
    pub status_known: bool,
}
impl PageState {
    pub fn has_room(&self) -> bool {
        !self.info.room_id.trim().is_empty()
    }
    /// The page shows a room or an account (`hasKnownPageIdentity`).
    pub fn known(&self) -> bool {
        self.has_room() || self.has_anchor
    }
}

/// The value at a gjson-style dotted path.
fn path<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(v, |v, key| match v {
        Value::Object(map) => map.get(key),
        Value::Array(items) => items.get(key.parse::<usize>().ok()?),
        _ => None,
    })
}

/// gjson's `Result.String()`: strings as is, numbers and booleans as text,
/// objects and arrays as JSON, null or missing as empty.
fn text(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

fn first(v: &Value, paths: &[&str]) -> String {
    paths
        .iter()
        .map(|p| text(path(v, p)))
        .find(|s| !s.is_empty())
        .unwrap_or_default()
}

fn parse(body: &str) -> Value {
    serde_json::from_str(body).unwrap_or(Value::Null)
}

/// Room identity from a web/enter body (`parseRoomInfo`).
pub fn room_enter(body: &str) -> Result<RoomInfo, &'static str> {
    let v = parse(body);
    let room_id = first(
        &v,
        &[
            "data.data.0.id_str",
            "data.data.0.id",
            "data.room.id_str",
            "data.room.id",
            "data.enter_room_id",
        ],
    );
    let push_id = first(
        &v,
        &[
            "data.user.id_str",
            "data.user.id",
            "data.data.0.owner_user_id_str",
            "data.data.0.owner.id_str",
            "data.data.0.owner.id",
            "data.room.owner_user_id_str",
            "data.room.owner.id_str",
            "data.room.owner.id",
        ],
    );
    if room_id.is_empty() || push_id.is_empty() {
        return Err("无法提取房间信息");
    }
    Ok(RoomInfo {
        room_id,
        push_id,
        live_name: first(
            &v,
            &[
                "data.user.nickname",
                "data.data.0.owner.nickname",
                "data.room.owner.nickname",
            ],
        ),
        title: first(&v, &["data.data.0.title", "data.room.title"]),
        avatar: first(
            &v,
            &[
                "data.user.avatar_thumb.url_list.2",
                "data.user.avatar_thumb.url_list.1",
                "data.user.avatar_thumb.url_list.0",
                "data.data.0.owner.avatar_thumb.url_list.2",
                "data.data.0.owner.avatar_thumb.url_list.1",
                "data.data.0.owner.avatar_thumb.url_list.0",
                "data.room.owner.avatar_thumb.url_list.2",
                "data.room.owner.avatar_thumb.url_list.1",
                "data.room.owner.avatar_thumb.url_list.0",
            ],
        ),
        anchor_only: false,
    })
}

/// Live status from a web/enter body: Some(live) when the status is present
/// (`parseLiveStatusFromRoomEnter`). Status 2 means live.
pub fn room_enter_live(body: &str) -> Option<bool> {
    let v = parse(body);
    ["data.data.0.status", "data.room.status"]
        .into_iter()
        .find_map(|p| match path(&v, p)? {
            Value::Number(n) => n.as_i64().map(|code| code == 2),
            Value::String(s) => s.parse::<i64>().ok().map(|code| code == 2),
            _ => None,
        })
}

/// The page body and its unescaped forms (`livePageStateCandidates`).
fn candidates(body: &str) -> Vec<String> {
    let mut out = vec![body.to_owned()];
    let mut decoded = body.to_owned();
    for _ in 0..6 {
        // One left-to-right pass trying the replacements in order, like strings.NewReplacer.
        let mut next = String::with_capacity(decoded.len());
        let mut rest = decoded.as_str();
        'scan: while !rest.is_empty() {
            for (from, to) in [
                (r"\\u0026", "&"),
                (r"&", "&"),
                (r#"\\\""#, "\""),
                (r#"\""#, "\""),
            ] {
                if let Some(after) = rest.strip_prefix(from) {
                    next.push_str(to);
                    rest = after;
                    continue 'scan;
                }
            }
            let ch = rest.chars().next().unwrap_or_default();
            next.push(ch);
            rest = &rest[ch.len_utf8()..];
        }
        if next == decoded {
            break;
        }
        decoded = next;
        out.push(decoded.clone());
    }
    out
}

/// The balanced JSON object starting at `open` (`jsonObjectAt`).
fn object_at(body: &str, open: usize) -> &str {
    let bytes = body.as_bytes();
    if bytes.get(open) != Some(&b'{') {
        return "";
    }
    let (mut depth, mut in_string, mut escaped) = (0i32, false, false);
    for (idx, &ch) in bytes.iter().enumerate().skip(open) {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == b'\\' {
                escaped = true;
            } else if ch == b'"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return &body[open..=idx];
                }
            }
            _ => (),
        }
    }
    ""
}

/// The object after the first `{` that follows `at`.
fn object_after(body: &str, at: usize) -> &str {
    body[at..]
        .find('{')
        .map_or("", |rel| object_at(body, at + rel))
}

fn has_room_identity(obj: &Value) -> bool {
    !first(obj, &["room.id_str", "room.id"]).is_empty()
}

fn has_anchor_identity(obj: &Value) -> bool {
    if !matches!(path(obj, "anchor"), Some(Value::Object(_))) {
        return false;
    }
    let named = first(
        obj,
        &[
            "anchor.id_str",
            "anchor.id",
            "anchor.sec_uid",
            "anchor.nickname",
            "anchor.avatar_thumb.url_list.0",
        ],
    );
    !named.is_empty() || text(path(obj, "anchor")).trim() != "{}"
}

/// The `roomInfo` object with a room, else the first one with an anchor
/// (`roomInfoObjectFromLivePageState`).
fn room_info_object(body: &str) -> Option<Value> {
    let mut anchor_only = None;
    for marker in [r#""roomStore":{"roomInfo":"#, r#""roomInfo":"#] {
        let mut from = 0;
        while let Some(rel) = body[from..].find(marker) {
            let at = from + rel + marker.len();
            let obj = object_after(body, at);
            if !obj.is_empty() {
                let v = parse(obj);
                if has_room_identity(&v) {
                    return Some(v);
                }
                if anchor_only.is_none() && has_anchor_identity(&v) {
                    anchor_only = Some(v);
                }
            }
            from = at;
        }
    }
    anchor_only
}

/// The first room object with an id_str (`roomObjectFromLivePageState`).
fn room_object(body: &str) -> Option<Value> {
    for marker in [
        r#""roomStore":{"roomInfo":{"room":"#,
        r#""roomInfo":{"room":"#,
        r#""room":"#,
    ] {
        let Some(idx) = body.find(marker) else {
            continue;
        };
        let obj = object_after(body, idx + marker.len());
        if obj.is_empty() {
            continue;
        }
        let v = parse(obj);
        if !text(path(&v, "id_str")).is_empty() {
            return Some(v);
        }
    }
    None
}

const AVATAR_PATHS: [&str; 3] = [
    "avatar_thumb.url_list.2",
    "avatar_thumb.url_list.1",
    "avatar_thumb.url_list.0",
];
fn prefixed(prefixes: &[&str], fields: &[&str]) -> Vec<String> {
    prefixes
        .iter()
        .flat_map(|p| fields.iter().map(move |f| format!("{p}{f}")))
        .collect()
}
fn first_of(v: &Value, paths: &[String]) -> String {
    first(v, &paths.iter().map(String::as_str).collect::<Vec<_>>())
}

/// Room metadata from the page's embedded state (`parseRoomInfoFromLivePage`).
fn room_info_from_page(body: &str) -> RoomInfo {
    for candidate in candidates(body) {
        if let Some(obj) = room_info_object(&candidate) {
            let info = RoomInfo {
                room_id: first(&obj, &["room.id_str", "room.id"]),
                push_id: first(
                    &obj,
                    &[
                        "anchor.id_str",
                        "anchor.id",
                        "room.owner.id_str",
                        "room.owner.id",
                        "room.owner_user_id_str",
                        "room.owner_user_id",
                        "owner.id_str",
                        "owner.id",
                    ],
                ),
                live_name: first(
                    &obj,
                    &["anchor.nickname", "room.owner.nickname", "owner.nickname"],
                ),
                title: first(&obj, &["room.title", "title"]),
                avatar: first_of(
                    &obj,
                    &prefixed(&["anchor.", "room.owner.", "owner."], &AVATAR_PATHS),
                ),
                anchor_only: has_anchor_identity(&obj) && !has_room_identity(&obj),
            };
            if info != RoomInfo::default() {
                return info;
            }
        }
        let Some(room) = room_object(&candidate) else {
            continue;
        };
        let info = RoomInfo {
            room_id: first(&room, &["id_str", "id"]),
            push_id: first(
                &room,
                &[
                    "owner.id_str",
                    "owner.id",
                    "owner_user_id_str",
                    "owner_user_id",
                ],
            ),
            live_name: first(&room, &["owner.nickname", "anchor.nickname"]),
            title: first(&room, &["title"]),
            avatar: first_of(&room, &prefixed(&["owner.", "anchor."], &AVATAR_PATHS)),
            anchor_only: false,
        };
        if info != RoomInfo::default() {
            return info;
        }
    }
    RoomInfo::default()
}

/// One match of the original's `livePageRoomStatusPattern`:
/// `"id_str"\s*:\s*"?(\d+)"?\s*,\s*"status"\s*:\s*(\d+)\s*,\s*"status_str"\s*:\s*"?([^",}]*)"?`
/// Returns the room id, the status digits and where the match ends.
fn status_match(s: &[u8], start: usize) -> Option<(&[u8], &[u8], usize)> {
    let space = |b: u8| matches!(b, b'\t' | b'\n' | b'\x0c' | b'\r' | b' ');
    let mut i = start;
    let lit = |i: &mut usize, text: &[u8]| -> Option<()> {
        s.get(*i..*i + text.len()).filter(|t| *t == text)?;
        *i += text.len();
        Some(())
    };
    let spaces = |i: &mut usize| {
        while s.get(*i).copied().is_some_and(space) {
            *i += 1;
        }
    };
    let quote = |i: &mut usize| {
        if s.get(*i) == Some(&b'"') {
            *i += 1;
        }
    };
    let digits = |i: &mut usize| -> Option<usize> {
        let from = *i;
        while s.get(*i).is_some_and(u8::is_ascii_digit) {
            *i += 1;
        }
        (*i > from).then_some(from)
    };
    lit(&mut i, b"\"id_str\"")?;
    spaces(&mut i);
    lit(&mut i, b":")?;
    spaces(&mut i);
    quote(&mut i);
    let id_from = digits(&mut i)?;
    let id = &s[id_from..i];
    quote(&mut i);
    spaces(&mut i);
    lit(&mut i, b",")?;
    spaces(&mut i);
    lit(&mut i, b"\"status\"")?;
    spaces(&mut i);
    lit(&mut i, b":")?;
    spaces(&mut i);
    let status_from = digits(&mut i)?;
    let status = &s[status_from..i];
    spaces(&mut i);
    lit(&mut i, b",")?;
    spaces(&mut i);
    lit(&mut i, b"\"status_str\"")?;
    spaces(&mut i);
    lit(&mut i, b":")?;
    spaces(&mut i);
    quote(&mut i);
    while s.get(i).is_some_and(|b| !matches!(b, b'"' | b',' | b'}')) {
        i += 1;
    }
    quote(&mut i);
    Some((id, status, i))
}

/// Every non-overlapping status match, left to right.
fn status_matches(s: &str) -> Vec<(String, String)> {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(rel) = s[at..].find("\"id_str\"") {
        let start = at + rel;
        match status_match(bytes, start) {
            Some((id, status, end)) => {
                out.push((
                    String::from_utf8_lossy(id).into_owned(),
                    String::from_utf8_lossy(status).into_owned(),
                ));
                at = end.max(start + 1);
            }
            None => at = start + 1,
        }
    }
    out
}

/// At least 10 digits after a marker, past quotes, colons and spaces (`digitsAfterMarker`).
fn digits_after(body: &str, marker: &str) -> String {
    let Some(idx) = body.find(marker) else {
        return String::new();
    };
    let bytes = body.as_bytes();
    let mut start = idx + marker.len();
    while bytes
        .get(start)
        .is_some_and(|b| matches!(b, b'"' | b'\'' | b':' | b' '))
    {
        start += 1;
    }
    let mut end = start;
    while bytes.get(end).is_some_and(u8::is_ascii_digit) {
        end += 1;
    }
    if end - start < 10 {
        return String::new();
    }
    body[start..end].to_owned()
}

fn room_id_from_page(body: &str) -> String {
    for candidate in candidates(body) {
        if let Some((id, _)) = status_matches(&candidate)
            .into_iter()
            .find(|(id, _)| !id.is_empty())
        {
            return id;
        }
    }
    [
        r#""room":{"id_str":""#,
        r#"\"room\":{\"id_str\":\""#,
        r#""room":{"id":"#,
        r#"\"room\":{\"id\":"#,
        "room_id=",
        "room_id%3D",
        r#""room_id":""#,
        r#"\"room_id\":\""#,
        r#""room_id":"#,
        r#""room_id_str":""#,
        r#"\"room_id_str\":\""#,
        r#""roomId":""#,
        r#"\"roomId\":\""#,
        r#""roomId":"#,
        "gift_effect_bg_",
    ]
    .iter()
    .map(|m| digits_after(body, m))
    .find(|v| !v.is_empty())
    .unwrap_or_default()
}

fn user_unique_id_from_page(body: &str) -> String {
    [
        "user_unique_id=",
        "user_unique_id%3D",
        r#""user_unique_id":""#,
        r#"\"user_unique_id\":\""#,
        r#""user_unique_id":"#,
    ]
    .iter()
    .map(|m| digits_after(body, m))
    .find(|v| !v.is_empty())
    .unwrap_or_default()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// Live status for a room on the page, when the page states it (`parseLiveStatusFromLivePage`).
fn live_from_page(body: &str, room_id: &str) -> Option<bool> {
    for candidate in candidates(body) {
        if let Some(room) = room_object(&candidate)
            && let Some(status) = path(&room, "status")
        {
            let id = first(&room, &["id_str", "id"]);
            if room_id.is_empty() || id == room_id || candidate.contains(room_id) {
                // gjson's Int(): numbers truncate, numeric strings parse, the rest is 0.
                let code = match status {
                    Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
                    Value::String(s) => s.parse().ok(),
                    Value::Bool(b) => Some(i64::from(*b)),
                    _ => None,
                };
                return Some(code == Some(2));
            }
        }
        for (id, status) in status_matches(&candidate) {
            if room_id.is_empty() || id == room_id {
                return Some(status == "2");
            }
        }
        if !room_id.is_empty()
            && let Some(idx) = candidate.find(room_id)
        {
            let bytes = candidate.as_bytes();
            let segment = &bytes[idx..(idx + 1200).min(bytes.len())];
            if contains(segment, br#""status":2"#) || contains(segment, br#"\"status\":2"#) {
                return Some(true);
            }
            if contains(segment, br#""status":"#) || contains(segment, br#"\"status\":"#) {
                return Some(false);
            }
        }
        let offline = ["直播已结束", "暂未开播", "未开播"]
            .iter()
            .any(|t| candidate.contains(t));
        if offline && (room_id.is_empty() || candidate.contains(room_id)) {
            return Some(false);
        }
    }
    None
}

/// Everything the original reads from a live page (`parseLivePageState`).
pub fn live_page(body: &str) -> PageState {
    let mut state = PageState {
        info: room_info_from_page(body),
        ..Default::default()
    };
    let info = &state.info;
    state.has_anchor = info.anchor_only
        || !info.live_name.trim().is_empty()
        || !info.avatar.trim().is_empty()
        || (info.room_id.is_empty() && !info.push_id.trim().is_empty());
    if state.info.room_id.is_empty() && !state.info.anchor_only {
        state.info.room_id = room_id_from_page(body);
    }
    state.user_unique_id = user_unique_id_from_page(body);
    if !state.user_unique_id.is_empty() {
        state.info.push_id = state.user_unique_id.clone();
    }
    if state.info.anchor_only || (!state.has_room() && state.has_anchor) {
        state.status_known = true;
    } else if state.has_room()
        && let Some(live) = live_from_page(body, &state.info.room_id)
    {
        state.live = live;
        state.status_known = true;
    }
    state
}
