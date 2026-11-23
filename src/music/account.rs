use super::Provider;
use crate::{api::Problem, audio, config, state::App};
use anyhow::{Context, Result, ensure};
use axum::{
    Json,
    extract::{Path, State},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{fs, io::ErrorKind, path::PathBuf, time::Duration};

fn path(app: &App, provider: Provider) -> PathBuf {
    app.dir.join(match provider {
        Provider::Qq => "qq-music.cookie",
        Provider::Netease => "netease-music.cookie",
    })
}

pub fn read(app: &App, provider: Provider) -> Result<Option<String>> {
    match fs::read_to_string(path(app, provider)) {
        Ok(cookie) => Ok(Some(cookie)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn field<'a>(cookie: &'a str, name: &str) -> Option<&'a str> {
    cookie
        .split(';')
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(key, value)| *key == name && !value.is_empty())
        .map(|(_, v)| v)
}

fn qq_user(cookie: &str) -> Option<&str> {
    let key = if field(cookie, "login_type") == Some("2") {
        "wxuin"
    } else {
        "uin"
    };
    field(cookie, key)
        .or_else(|| field(cookie, "wxuin"))
        .map(|id| id.trim_start_matches('o'))
        .filter(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))
}

fn qq_key(cookie: &str) -> Option<&str> {
    field(cookie, "qqmusic_key").or_else(|| field(cookie, "qm_keyst"))
}

#[derive(Deserialize)]
pub struct Input {
    cookie: String,
}

pub async fn status(
    State(app): State<App>,
    Path(provider): Path<Provider>,
) -> Result<Json<Value>, Problem> {
    Ok(Json(json!({"configured":read(&app,provider)?.is_some()})))
}

fn normalize(input: &str, provider: Provider) -> Result<&str> {
    let raw = input.trim();
    let raw = raw
        .strip_prefix("Cookie:")
        .or_else(|| raw.strip_prefix("cookie:"))
        .unwrap_or(raw)
        .trim();
    ensure!(
        raw.len() <= 16384 && raw.bytes().all(|b| (32..127).contains(&b)),
        "Cookie 格式无效"
    );
    ensure!(
        match provider {
            Provider::Netease => field(raw, "MUSIC_U").is_some(),
            Provider::Qq => qq_user(raw).is_some() && qq_key(raw).is_some(),
        },
        "请粘贴所选平台已登录账号的完整 Cookie"
    );
    Ok(raw)
}

pub async fn save(
    State(app): State<App>,
    Path(provider): Path<Provider>,
    Json(input): Json<Input>,
) -> Result<Json<Value>, Problem> {
    config::atomic(
        &path(&app, provider),
        normalize(&input.cookie, provider)?.as_bytes(),
    )?;
    Ok(Json(json!({"configured":true})))
}

pub async fn clear(
    State(app): State<App>,
    Path(provider): Path<Provider>,
) -> Result<Json<Value>, Problem> {
    if let Err(e) = fs::remove_file(path(&app, provider))
        && e.kind() != ErrorKind::NotFound
    {
        return Err(anyhow::Error::from(e).into());
    }
    Ok(Json(json!({"configured":false})))
}

// Keep account cookies on the provider's HTTPS origin, including when a custom QQMusicApi is configured.
pub async fn qq_playback(app: &App, id: &str, cookie: &str) -> Result<String> {
    let user = qq_user(cookie).context("Cookie 格式无效")?;
    let payload = json!({
        "comm":{"uin":user,"format":"json","ct":24,"cv":0,"authst":qq_key(cookie)},
        "req_0":{"module":"vkey.GetVkeyServer","method":"CgiGetVkey","param":{
            "guid":(uuid::Uuid::new_v4().as_u128()%10000000000).to_string(),
            "songmid":[id],"songtype":[0],"uin":user,"loginflag":1,"platform":"20"
        }}
    });
    let response = app
        .http
        .post("https://u.y.qq.com/cgi-bin/musicu.fcg")
        .header("Referer", "https://y.qq.com/")
        .header("Cookie", cookie)
        .json(&payload)
        .timeout(Duration::from_secs(15))
        .send()
        .await?
        .error_for_status()?;
    let value: Value = serde_json::from_slice(&audio::limited(response, 1024 * 1024).await?)?;
    let data = &value["req_0"]["data"];
    let file = data
        .pointer("/midurlinfo/0/purl")
        .and_then(Value::as_str)
        .filter(|p| !p.is_empty())
        .context("QQ 音乐未提供音源，请检查 Cookie 和歌曲播放权限")?;
    let base = data
        .pointer("/sip/0")
        .and_then(Value::as_str)
        .context("平台未提供可播放音源")?;
    Ok(url::Url::parse(base)?.join(file)?.to_string())
}
