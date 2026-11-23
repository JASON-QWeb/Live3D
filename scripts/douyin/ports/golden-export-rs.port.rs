//! Wrappers over Vorela's `src/douyin/params.rs` and `page.rs`, compiled from those files.

#![forbid(unsafe_code)]

#[path = "LIVE3D/src/douyin/page.rs"]
pub mod page;
#[path = "LIVE3D/src/douyin/params.rs"]
pub mod params;
#[path = "LIVE3D/src/douyin/sign.rs"]
pub mod sign;

use params::Screen;

pub fn golden_ws_stub(room_id: String, user_unique_id: String) -> Result<String, String> {
    Ok(params::websocket_stub(&room_id, &user_unique_id))
}

#[expect(clippy::too_many_arguments, reason = "mirrors the Go wrapper")]
pub fn golden_ws_query(
    room_id: String,
    push_id: String,
    user_agent: String,
    cursor: String,
    internal_ext: String,
    signature: String,
    width: i64,
    height: i64,
) -> Result<String, String> {
    Ok(params::websocket_query(
        &room_id,
        &push_id,
        &user_agent,
        &cursor,
        &internal_ext,
        &signature,
        Screen { width, height },
    ))
}

pub fn golden_im_fetch_query(
    room_id: String,
    push_id: String,
    user_agent: String,
    ms_token: String,
    width: i64,
    height: i64,
) -> Result<String, String> {
    Ok(params::im_fetch_query(
        &room_id,
        &push_id,
        &user_agent,
        &ms_token,
        Screen { width, height },
    ))
}

pub fn golden_room_enter_query(
    live_id: String,
    room_id: String,
    user_agent: String,
    ms_token: String,
    live_known: bool,
    width: i64,
    height: i64,
) -> Result<String, String> {
    let room = if live_known { room_id.as_str() } else { "" };
    Ok(params::room_enter_query(
        &live_id,
        room,
        &user_agent,
        &ms_token,
        Screen { width, height },
    ))
}

pub fn golden_internal_ext(
    room_id: String,
    user_unique_id: String,
    now_ms: i64,
) -> Result<String, String> {
    Ok(params::default_internal_ext(
        &room_id,
        &user_unique_id,
        now_ms,
    ))
}

pub fn golden_push_url(candidate: String) -> Result<String, String> {
    Ok(params::push_url(&candidate))
}

pub fn golden_ping_interval(header: String) -> Result<i64, String> {
    Ok(params::ping_interval(&header))
}

pub fn golden_chrome_version(user_agent: String) -> Result<String, String> {
    Ok(params::chrome_version(&user_agent).to_owned())
}

pub fn golden_query_escape(value: String) -> Result<String, String> {
    Ok(format!(
        "{}\t{}",
        params::query_escape_value(&value),
        params::query_escape(&value)
    ))
}

pub fn golden_validate_live_id(value: String) -> Result<String, String> {
    params::live_id(&value)
        .map(str::to_owned)
        .ok_or_else(|| "直播间标识无效".to_owned())
}

fn snapshot(info: &page::RoomInfo) -> String {
    [
        &info.room_id,
        &info.push_id,
        &info.live_name,
        &info.title,
        &info.avatar,
        &info.anchor_only.to_string(),
    ]
    .map(String::as_str)
    .join("|")
}

pub fn golden_room_enter(body: String) -> Result<String, String> {
    let info = page::room_enter(&body).map_err(str::to_owned)?;
    let live = page::room_enter_live(&body);
    Ok(format!(
        "{}|live={}|known={}",
        snapshot(&info),
        live == Some(true),
        live.is_some()
    ))
}

pub fn golden_live_page(body: String) -> Result<String, String> {
    let s = page::live_page(&body);
    Ok(format!(
        "{}|uid={}|anchor={}|live={}|known={}|valid={}",
        snapshot(&s.info),
        s.user_unique_id,
        s.has_anchor,
        s.live,
        s.status_known,
        s.known()
    ))
}
