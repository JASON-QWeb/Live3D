use crate::{
    audio::{checked, limited},
    config::{Settings, image_uri},
    state::{App, Rtc},
};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::time::Duration;

const API: &str = "https://api.vidu.cn";
pub fn id(v: &Value) -> Option<String> {
    v.as_str()
        .map(str::to_owned)
        .or_else(|| v.as_u64().map(|n| n.to_string()))
}
pub fn valid_id(id: &str) -> Result<()> {
    ensure!(
        !id.is_empty()
            && id.len() <= 160
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
        "云端会话标识无效"
    );
    Ok(())
}
pub struct Created {
    pub id: String,
    pub rtc: Option<Rtc>,
}
pub fn create_body(app: &App, c: &Settings) -> Result<Value> {
    let mut b = json!({"model":"vidu-s2","resolution":c.resolution,
        "managed_rtc":{"provider":"trtc","enable_recording":c.recording},
        "action":{"enabled":true,"library_id":if c.action_library.trim().is_empty(){"0"}else{c.action_library.trim()}}});
    if !c.avatar_id.trim().is_empty() {
        b["avatar_id"] = json!(c.avatar_id.trim());
    } else {
        let data = std::fs::read(app.dir.join("avatar.image"))
            .context("请上传自己的角色图或填写 avatar_id")?;
        b["image_uri"] = json!(image_uri(&data)?);
    }
    Ok(b)
}
pub async fn create(app: &App, key: &str, body: Value) -> Result<reqwest::Response> {
    let r = app
        .http
        .post(format!("{API}/live/s_avatar/component"))
        .timeout(Duration::from_secs(60))
        .header("Authorization", format!("Token {key}"))
        .json(&body)
        .send()
        .await
        .map_err(|e| e.without_url())?;
    Ok(r)
}
pub async fn created(r: reqwest::Response) -> Result<Created> {
    let v: Value = serde_json::from_slice(&limited(checked(r, "Vidu").await?, 256_000).await?)?;
    let live_id = v
        .pointer("/live/id")
        .and_then(id)
        .context("Vidu 未返回可核对的会话 ID")?;
    valid_id(&live_id)?;
    let rtc = v
        .get("managed_rtc")
        .filter(|o| o["provider"] == "trtc")
        .and_then(|o| {
            Some(Rtc {
                app_id: id(&o["app_id"])?,
                channel_id: id(&o["channel_id"])?,
                user_id: id(&o["user_id"])?,
                token: o["token"].as_str()?.to_owned(),
            })
        });
    Ok(Created { id: live_id, rtc })
}
pub async fn query(app: &App, key: &str, live_id: &str, recording: bool) -> Result<Value> {
    valid_id(live_id)?;
    let suffix = if recording { "/recording" } else { "" };
    let r = app
        .http
        .get(format!("{API}/live/v1/lives/{live_id}{suffix}"))
        .header("Authorization", format!("Token {key}"))
        .timeout(Duration::from_secs(8))
        .send()
        .await
        .map_err(|e| e.without_url())?;
    Ok(serde_json::from_slice(
        &limited(checked(r, "Vidu").await?, 256_000).await?,
    )?)
}

pub async fn balance(app: &App) -> Result<Value> {
    let k = crate::config::key("vidu")?.context("请先保存 Vidu 密钥")?;
    let r = app
        .http
        .get(format!("{API}/ent/v2/credits"))
        .query(&[("show_detail", "false")])
        .header("Authorization", format!("Token {k}"))
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| e.without_url())?;
    let body: Value = serde_json::from_slice(&limited(checked(r, "Vidu").await?, 128_000).await?)?;
    let items = body
        .get("remains")
        .or_else(|| body.get("remaining_credits"))
        .and_then(Value::as_array)
        .context("平台未返回余额明细")?;
    Ok(
        json!({"balances":items.iter().map(|v|json!({"type":v["type"],"credits":v["credit_remain"]})).collect::<Vec<_>>()}),
    )
}
