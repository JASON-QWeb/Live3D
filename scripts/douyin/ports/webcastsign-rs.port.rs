//! Wrappers over Vorela's `src/douyin/sign.rs`, compiled from that file.

#![forbid(unsafe_code)]

#[path = "LIVE3D/src/douyin/sign.rs"]
pub mod sign;

fn byte(v: i64) -> Result<u8, String> {
    u8::try_from(v).map_err(|_| format!("{v} is not a byte"))
}

/// Port of `SignWithValues` (`internal/webcastsign/native.go:86`).
pub fn sign_with_values(
    x_ms_stub: String,
    counter: i64,
    random_flag: bool,
    payload_random: i64,
    key_random: i64,
) -> Result<String, String> {
    sign::websocket_signature_with(
        &x_ms_stub,
        byte(counter)?,
        random_flag,
        byte(payload_random)?,
        byte(key_random)?,
    )
}
