pub mod account;
use crate::{audio, library::BgmTrack, state::App};
use anyhow::{Context, Result, ensure};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
use url::Url;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Netease,
    Qq,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Source {
    pub provider: Provider,
    pub key: String,
}

impl Source {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.key.is_empty()
                && self.key.len() <= 32
                && self
                    .key
                    .bytes()
                    .all(|b| if self.provider == Provider::Netease {
                        b.is_ascii_digit()
                    } else {
                        b.is_ascii_alphanumeric()
                    }),
            "无效的歌曲 ID"
        );
        Ok(())
    }
}

async fn query(
    app: &App,
    provider: Provider,
    route: &str,
    params: &[(&str, &str)],
) -> Result<Value> {
    let (base, referer) = match provider {
        Provider::Netease => ("https://music.163.com/api".into(), "https://music.163.com/"),
        Provider::Qq => (
            std::env::var("VORELA_QQ_MUSIC_API").unwrap_or_else(|_| "http://127.0.0.1:3300".into()),
            "https://y.qq.com/",
        ),
    };
    let mut request = app
        .http
        .get(format!("{}{route}", base.trim_end_matches('/')))
        .query(params)
        .header("Referer", referer)
        .header("User-Agent", "Mozilla/5.0")
        .timeout(Duration::from_secs(15));
    if provider == Provider::Netease
        && let Some(cookie) = account::read(app, provider)?
    {
        request = request.header("Cookie", cookie);
    }
    let response = request
        .send()
        .await
        .context(match provider {
            Provider::Netease => "连接网易云音乐失败",
            Provider::Qq => "连接 QQMusicApi 失败，请先启动 QQ 音乐接口服务",
        })?
        .error_for_status()?;
    let v: Value = serde_json::from_slice(&audio::limited(response, 12 * 1024 * 1024).await?)?;
    let success = match provider {
        Provider::Netease => v["code"] == 200,
        Provider::Qq => v["result"] == 100,
    };
    ensure!(success, "音乐平台未能完成请求");
    Ok(v)
}

fn tracks(v: &Value, provider: Provider) -> Result<Vec<BgmTrack>> {
    v.as_array()
        .context("音乐平台未返回曲目列表")?
        .iter()
        .map(|song| {
            let (key, title, artists, duration) = match provider {
                Provider::Netease => (
                    song["id"].as_u64().context("歌曲缺少 ID")?.to_string(),
                    &song["name"],
                    &song["artists"],
                    song["duration"].as_u64(),
                ),
                Provider::Qq => (
                    song["songmid"].as_str().context("歌曲缺少 MID")?.to_owned(),
                    &song["songname"],
                    &song["singer"],
                    song["interval"].as_u64().map(|v| v.saturating_mul(1000)),
                ),
            };
            let name = title.as_str().context("歌曲缺少名称")?;
            let artists = artists
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|a| a["name"].as_str())
                .collect::<Vec<_>>()
                .join(" / ");
            let source = Source { provider, key };
            source.validate()?;
            Ok(BgmTrack {
                id: Uuid::new_v4(),
                title: if artists.is_empty() {
                    name.into()
                } else {
                    format!("{name} · {artists}")
                },
                duration_ms: duration.unwrap_or_default() as usize,
                source_ext: String::new(),
                online: Some(source),
            })
        })
        .collect()
}

pub async fn search(app: &App, provider: Provider, term: &str) -> Result<Vec<BgmTrack>> {
    let term = term.trim();
    ensure!(
        !term.is_empty() && term.chars().count() <= 100,
        "请输入 1 至 100 字的歌曲关键词"
    );
    let v = match provider {
        Provider::Netease => {
            query(
                app,
                provider,
                "/search/get",
                &[("s", term), ("type", "1"), ("limit", "10")],
            )
            .await?
        }
        Provider::Qq => {
            query(
                app,
                provider,
                "/search",
                &[("key", term), ("pageSize", "10"), ("pageNo", "1")],
            )
            .await?
        }
    };
    if v.pointer("/result/songCount").and_then(Value::as_u64) == Some(0) {
        return Ok(vec![]);
    }
    tracks(
        match provider {
            Provider::Netease => &v["result"]["songs"],
            Provider::Qq => &v["data"]["list"],
        },
        provider,
    )
}

fn playlist_id(input: &str, provider: Provider) -> Result<String> {
    let valid = |s: &str| !s.is_empty() && s.len() <= 20 && s.bytes().all(|c| c.is_ascii_digit());
    let input = input.trim();
    if valid(input) {
        return Ok(input.into());
    }
    let url = Url::parse(&input.replace("/#/", "/")).context("请输入歌单详情链接或数字 ID")?;
    let domain = match provider {
        Provider::Netease => "music.163.com",
        Provider::Qq => "y.qq.com",
    };
    ensure!(
        url.domain() == Some(domain),
        "链接与所选平台不匹配，请粘贴歌单详情页链接或数字 ID"
    );
    let id = url
        .query_pairs()
        .find(|(k, v)| matches!(k.as_ref(), "id" | "disstid") && valid(v))
        .map(|(_, v)| v.into_owned())
        .or_else(|| {
            url.path_segments()?
                .next_back()?
                .trim_end_matches(".html")
                .parse::<u64>()
                .ok()
                .map(|v| v.to_string())
        });
    id.context("链接中没有歌单 ID")
}

pub async fn playlist(app: &App, provider: Provider, input: &str) -> Result<Vec<BgmTrack>> {
    let id = playlist_id(input, provider)?;
    let (route, path) = match provider {
        Provider::Netease => ("/playlist/detail", "/result/tracks"),
        Provider::Qq => ("/songlist", "/data/songlist"),
    };
    let v = query(app, provider, route, &[("id", &id)]).await?;
    let items = tracks(v.pointer(path).context("找不到该歌单")?, provider)?;
    ensure!(!items.is_empty(), "歌单为空或当前不可访问");
    Ok(items)
}

pub async fn audio_bytes(app: &App, source: &Source) -> Result<(&'static str, Bytes)> {
    source.validate()?;
    let link = match source.provider {
        Provider::Netease => query(
            app,
            source.provider,
            "/song/enhance/player/url",
            &[("ids", &format!("[{}]", source.key)), ("br", "128000")],
        )
        .await?
        .pointer("/data/0/url")
        .and_then(Value::as_str)
        .map(str::to_owned),
        Provider::Qq => match account::read(app, Provider::Qq)? {
            Some(cookie) => Some(account::qq_playback(app, &source.key, &cookie).await?),
            None => query(app, source.provider, "/song/url", &[("id", &source.key)])
                .await?
                .get("data")
                .and_then(Value::as_str)
                .map(str::to_owned),
        },
    };
    let mut url = Url::parse(&link.context("平台未提供可播放音源")?)?;
    let host = url.domain().context("无效的音乐地址")?;
    ensure!(
        (match source.provider {
            Provider::Netease =>
                host.ends_with(".music.126.net") || host.ends_with(".music.163.com"),
            Provider::Qq => host.ends_with(".qq.com"),
        }) && matches!(url.scheme(), "http" | "https")
            && url.port().is_none(),
        "非平台音频地址"
    );
    url.set_scheme("https")
        .map_err(|_| anyhow::anyhow!("无效的音乐地址"))?;
    let res = app
        .http
        .get(url)
        .timeout(Duration::from_secs(50))
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(reqwest::Error::without_url)?;
    let data = audio::limited(res, audio::MAX_AUDIO).await?;
    let ext = audio::detect_audio_ext(&data, None).context("平台返回了无法识别的音频")?;
    Ok((audio::mime_for_ext(ext), data.into()))
}
