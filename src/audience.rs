use crate::{
    audio::{checked, limited},
    config::key,
    speech::{self, Job},
    state::{App, lock},
};
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::Value;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Incoming {
    pub id: String,
    pub author: String,
    pub text: String,
    #[serde(default)]
    pub gift: bool,
}
pub fn receive(app: &App, input: Incoming, source: &str) -> Result<bool> {
    let author = input.author.trim();
    let text = input.text.trim();
    ensure!(
        !input.id.is_empty()
            && input.id.len() <= 250
            && !author.is_empty()
            && author.chars().count() <= 80,
        "消息 ID 或作者无效"
    );
    ensure!(
        !text.is_empty() && text.chars().count() <= 600,
        "弹幕长度须在 1 至 600 字内"
    );
    {
        let id = format!("{source}/{}", input.id);
        let mut seen = lock(&app.seen);
        if seen.contains(&id) {
            return Ok(false);
        }
        if seen.len() == 2048 {
            seen.pop_front();
        }
        seen.push_back(id);
    }
    app.event(
        if input.gift { "礼物" } else { "弹幕" },
        format!("{author} · {text}"),
    );
    let live = {
        let mut v = lock(&app.view);
        v.metrics.inputs += 1;
        v.phase == "live"
    };
    if live {
        let cfg = app.config();
        let requested = if cfg.song_requests {
            text.strip_prefix("点歌")
                .map(|s| s.trim().trim_start_matches('：').trim())
                .filter(|s| !s.is_empty())
        } else {
            None
        };
        let job = if let Some(title) = requested {
            crate::library::list(app)?
                .into_iter()
                .find(|t| t.title == title)
                .map(|t| Job::Track(t.id))
        } else {
            match cfg.audience_mode.as_str() {
                "read" => Some(Job::Speak(text.into())),
                "reply" => Some(Job::Reply(text.into())),
                _ => None,
            }
        };
        if let Some(j) = job
            && let Ok(run) = app.active()
            && let Err(e) = speech::enqueue_in(app, &run, j, input.gift)
        {
            app.event("弹幕", format!("未排入播出：{e}"));
        }
    }
    Ok(true)
}
async fn fetch(app: &App, path: &str, params: &[(&str, &str)], key: &str) -> Result<Value> {
    let r = app
        .http
        .get(format!("https://www.googleapis.com/youtube/v3/{path}"))
        .query(params)
        .header("X-Goog-Api-Key", key)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| e.without_url())?;
    let b = limited(checked(r, "YouTube").await?, 2_000_000).await?;
    Ok(serde_json::from_slice(&b)?)
}
fn youtube_message(item: &Value) -> Option<Incoming> {
    let text = item.pointer("/snippet/displayMessage")?.as_str()?.trim();
    if text.is_empty() {
        return None;
    }
    Some(Incoming {
        gift: matches!(
            item.pointer("/snippet/type").and_then(Value::as_str),
            Some("superChatEvent" | "superStickerEvent" | "membershipGiftingEvent")
        ),
        id: item["id"].as_str()?.into(),
        author: item.pointer("/authorDetails/displayName")?.as_str()?.into(),
        text: text.into(),
    })
}
async fn listen(app: &App, video: &str, credential: &str) -> Result<()> {
    let info = fetch(
        app,
        "videos",
        &[("part", "liveStreamingDetails"), ("id", video)],
        credential,
    )
    .await?;
    let chat = info
        .pointer("/items/0/liveStreamingDetails/activeLiveChatId")
        .and_then(Value::as_str)
        .context("该视频未开放实时聊天或尚未开播")?;
    lock(&app.view).input = "YouTube 已连接".into();
    let mut cursor = String::new();
    let mut fails = 0;
    loop {
        let mut query = vec![
            ("liveChatId", chat),
            ("part", "snippet,authorDetails"),
            ("maxResults", "200"),
        ];
        if !cursor.is_empty() {
            query.push(("pageToken", &cursor));
        }
        let page = match fetch(app, "liveChat/messages", &query, credential).await {
            Ok(v) => {
                fails = 0;
                v
            }
            Err(e) if fails < 3 => {
                fails += 1;
                app.event("弹幕", format!("{e}，稍后重试"));
                tokio::time::sleep(Duration::from_secs(5 * fails)).await;
                continue;
            }
            Err(e) => return Err(e),
        };
        if !cursor.is_empty() {
            // The first response contains backlog from before connection.
            for item in page["items"].as_array().into_iter().flatten() {
                if let Some(message) = youtube_message(item) {
                    let _ = receive(app, message, "youtube");
                }
            }
        }
        if page.get("offlineAt").is_some() {
            break;
        }
        cursor = page["nextPageToken"]
            .as_str()
            .context("YouTube 未返回下一页游标")?
            .into();
        let wait = page["pollingIntervalMillis"]
            .as_u64()
            .unwrap_or(5000)
            .max(1000);
        tokio::time::sleep(Duration::from_millis(wait)).await;
    }
    Ok(())
}
pub async fn youtube(app: App, stop: CancellationToken) {
    let c = app.config();
    if c.youtube_video.is_empty() {
        return;
    }
    let work = async {
        let k = key("youtube")?.context("请配置 YouTube Data API 密钥")?;
        listen(&app, &c.youtube_video, &k).await
    };
    tokio::select! {
        _ = stop.cancelled() => (),
        result = work => {
            let msg = match result {
                Ok(()) => "YouTube 聊天已结束".into(),
                Err(e) => e.to_string(),
            };
            lock(&app.view).input = msg.clone();
            app.event("弹幕", msg);
        }
    }
}
