//! Wrappers over Vorela's `src/douyin/sign.rs`, compiled from that file.

#![forbid(unsafe_code)]

#[path = "LIVE3D/src/douyin/sign.rs"]
pub mod sign;

/// Port of `AbSign` (`sign/ab_sign.go`): `AbSignAt` at the current time.
pub fn ab_sign(url_search_params: String, user_agent: String) -> Result<String, String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?;
    Ok(sign::a_bogus_at(
        &url_search_params,
        &user_agent,
        now.as_millis() as i64,
    ))
}

/// Port of `AbSignAt`.
pub fn ab_sign_at(
    url_search_params: String,
    user_agent: String,
    start_time: i64,
) -> Result<String, String> {
    Ok(sign::a_bogus_at(
        &url_search_params,
        &user_agent,
        start_time,
    ))
}

/// Port of `SM3Hex`.
pub fn sm3_hex(data: String) -> Result<String, String> {
    Ok(sign::sm3_hex(data.as_bytes()))
}
