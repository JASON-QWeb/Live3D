use crate::{
    audio::{checked, limited},
    config::{key, now},
    director,
    replies::{Chat, Event},
    speech::{self, Job},
    state::{App, lock},
};
use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use serde_json::Value;
use std::{sync::atomic::Ordering, time::Duration};
use tokio_util::sync::CancellationToken;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Incoming {
    pub id: String,
    pub author: String,
    #[serde(default)]
    pub viewer_id: Option<String>,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub gift: bool,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub platform: Option<String>,
    #[serde(default)]
    pub gift_name: Option<String>,
    #[serde(default)]
    pub gift_count: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EventKind {
    Chat,
    Gift,
    Join,
}

pub fn has_english(name: &str) -> bool {
    name.chars().any(|c| c.is_ascii_alphabetic())
}

pub fn sanitize_author(author: &str, is_english_live: bool) -> Option<String> {
    let trimmed = author.trim();
    if trimmed.is_empty() {
        return None;
    }
    if !is_english_live && has_english(trimmed) {
        return None;
    }
    Some(trimmed.to_owned())
}

fn platform_label(platform: Option<&str>, source: &str) -> Result<&'static str> {
    let raw = platform
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(source);
    match raw.to_ascii_lowercase().as_str() {
        "douyin" => Ok("抖音"),
        "tiktok" => Ok("TikTok"),
        "youtube" => Ok("YouTube"),
        "local" | "simulate" => Ok("本机"),
        _ => bail!("平台仅支持 douyin、tiktok、youtube 或 local"),
    }
}

fn resolve_kind(input: &Incoming) -> Result<EventKind> {
    match input
        .kind
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some("join") => Ok(EventKind::Join),
        Some("gift") => Ok(EventKind::Gift),
        Some("chat") => Ok(if input.gift {
            EventKind::Gift
        } else {
            EventKind::Chat
        }),
        None => Ok(if input.gift {
            EventKind::Gift
        } else {
            EventKind::Chat
        }),
        Some(_) => bail!("事件类型仅支持 chat、gift、join"),
    }
}

fn format_gift_desc(gift_name: &str, count: u32, is_english: bool) -> String {
    if count > 1 {
        if is_english {
            format!("{gift_name} x{count}")
        } else {
            format!("{gift_name}×{count}")
        }
    } else {
        gift_name.to_owned()
    }
}

fn format_spoken_gift_desc(gift_name: &str, count: u32, is_english: bool) -> String {
    if count > 1 {
        if is_english {
            format!("{count} {gift_name}")
        } else {
            format!("{count}个{gift_name}")
        }
    } else {
        gift_name.to_owned()
    }
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
    let kind = resolve_kind(&input)?;
    let plat = platform_label(input.platform.as_deref(), source)?;

    let gift_name_clean = input
        .gift_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let gift_count = input.gift_count.unwrap_or(1).max(1);

    match kind {
        EventKind::Chat => {
            ensure!(
                !text.is_empty() && text.chars().count() <= 600,
                "弹幕长度须在 1 至 600 字内"
            );
        }
        EventKind::Gift => {
            let name = gift_name_clean.unwrap_or(text);
            ensure!(
                !name.is_empty() && name.chars().count() <= 200 && text.chars().count() <= 600,
                "礼物名称或留言无效"
            );
        }
        EventKind::Join => {
            ensure!(text.chars().count() <= 200, "进房附加信息过长");
        }
    }

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

    let cfg = app.config();
    let live = {
        let mut v = lock(&app.view);
        v.metrics.inputs += 1;
        v.phase == "live"
    };

    match kind {
        EventKind::Join => {
            if !cfg.is_english() && has_english(author) {
                app.event(
                    format!("{plat}·进房"),
                    format!("{author}（中文环境已跳过英文用户名）"),
                );
                return Ok(true);
            }
            app.event(format!("{plat}·进房"), author.to_owned());
            if live && cfg.welcome_joins && cfg.tts_mode != "system" {
                if let Ok(run) = app.active() {
                    let pending = lock(&app.view).pending;
                    let current = now();
                    let last_join = run.last_join_welcome_ms.load(Ordering::Relaxed);
                    let last_act = run.last_activity_ms.load(Ordering::Relaxed);
                    let join_cd = cfg.join_welcome_gap as u64 * 1000;
                    let act_gap = cfg.utterance_gap as u64 * 1000;
                    if !run.singing.load(Ordering::Relaxed)
                        && !run.busy.load(Ordering::Relaxed)
                        && pending == 0
                        && current.saturating_sub(last_join) >= join_cd
                        && current.saturating_sub(last_act) >= act_gap
                        && run
                            .last_join_welcome_ms
                            .compare_exchange(
                                last_join,
                                current,
                                Ordering::SeqCst,
                                Ordering::Relaxed,
                            )
                            .is_ok()
                    {
                        if let Err(e) = speech::enqueue_in(
                            app,
                            &run,
                            Job::AutoReply {
                                event: Event::Join(author.to_owned()),
                                label: format!("进房欢迎 · {author}"),
                            },
                            false,
                        ) {
                            app.event("进房", format!("未排入欢迎：{e}"));
                        }
                    }
                }
            }
        }
        EventKind::Gift => {
            let gname = gift_name_clean.unwrap_or(text);
            let gift_desc = format_gift_desc(gname, gift_count, cfg.is_english());
            let spoken_gift_desc = format_spoken_gift_desc(gname, gift_count, cfg.is_english());
            let detail = if !text.is_empty() && text != gname {
                format!("{author} · {gift_desc} · {text}")
            } else {
                format!("{author} · {gift_desc}")
            };
            app.event(format!("{plat}·礼物"), detail);
            if live {
                let safe_author = sanitize_author(author, cfg.is_english());
                if let Ok(run) = app.active() {
                    if director::try_buffer_song_gift(
                        &run,
                        safe_author.clone(),
                        gname.to_owned(),
                        gift_count,
                    ) {
                        app.event(
                            "礼物暂存",
                            format!("正在唱歌，{gift_desc} 已暂存至唱完后统一答谢"),
                        );
                        return Ok(true);
                    }
                    let job = if cfg.auto_thank_gifts {
                        if cfg.tts_mode != "system" {
                            Some(Job::AutoReply {
                                event: Event::Gift {
                                    viewer: safe_author,
                                    name: gname.to_owned(),
                                    description: spoken_gift_desc,
                                    count: gift_count,
                                    comment: (!text.is_empty()
                                        && text != gname
                                        && text != gift_desc)
                                        .then(|| text.to_owned()),
                                },
                                label: format!("感谢礼物 · {gift_desc}"),
                            })
                        } else {
                            None
                        }
                    } else if cfg.audience_mode == "read" {
                        let line = match safe_author {
                            Some(name) if cfg.is_english() => {
                                format!("Thank you {name} for {spoken_gift_desc}!")
                            }
                            Some(name) => format!("谢谢{name}送的{spoken_gift_desc}"),
                            None if cfg.is_english() => {
                                format!("Thank you for {spoken_gift_desc}!")
                            }
                            None => format!("谢谢老板送的{spoken_gift_desc}"),
                        };
                        Some(Job::Speak(line))
                    } else {
                        None
                    };
                    if let Some(j) = job
                        && let Err(e) = speech::enqueue_in(app, &run, j, true)
                    {
                        app.event("礼物", format!("未排入感谢：{e}"));
                    }
                }
            }
        }
        EventKind::Chat => {
            app.event(format!("{plat}·弹幕"), format!("{author} · {text}"));
            if live {
                let requested = if cfg.song_requests {
                    text.strip_prefix("点歌")
                        .or_else(|| text.strip_prefix("!song "))
                        .or_else(|| text.strip_prefix("song "))
                        .map(|s| {
                            s.trim()
                                .trim_start_matches('：')
                                .trim_start_matches(':')
                                .trim()
                        })
                        .filter(|s| !s.is_empty())
                } else {
                    None
                };
                let job = if let Some(title) = requested {
                    crate::library::list(app)?
                        .into_iter()
                        .find(|t| {
                            t.title == title
                                || crate::config::resolve_song_title(&cfg, &t.title)
                                    .eq_ignore_ascii_case(title)
                        })
                        .map(|t| Job::Track(t.id))
                } else {
                    let safe_author = sanitize_author(author, cfg.is_english());
                    match cfg.audience_mode.as_str() {
                        "read" => Some(Job::Speak(text.into())),
                        "reply" => Some(Job::Reply(Chat {
                            text: text.into(),
                            viewer: safe_author,
                            viewer_id: input
                                .viewer_id
                                .as_deref()
                                .map(str::trim)
                                .filter(|id| !id.is_empty() && id.len() <= 250)
                                .map(|id| format!("{source}/{plat}/{id}")),
                        })),
                        _ => None,
                    }
                };
                if let Some(j) = job
                    && let Ok(run) = app.active()
                    && let Err(e) = speech::enqueue_in(app, &run, j, false)
                {
                    app.event("弹幕", format!("未排入播出：{e}"));
                }
            }
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
    let kind_raw = item.pointer("/snippet/type").and_then(Value::as_str);
    let gift = matches!(
        kind_raw,
        Some("superChatEvent" | "superStickerEvent" | "membershipGiftingEvent")
    );
    let gift_name = if gift {
        item.pointer("/snippet/superStickerDetails/superStickerMetadata/altText")
            .and_then(Value::as_str)
            .or_else(|| {
                item.pointer("/snippet/superChatDetails/amountDisplayString")
                    .and_then(Value::as_str)
            })
            .map(|s| s.to_owned())
            .or_else(|| Some("Super Chat".into()))
    } else {
        None
    };
    Some(Incoming {
        gift,
        kind: Some(if gift { "gift" } else { "chat" }.into()),
        platform: Some("youtube".into()),
        gift_name,
        gift_count: gift.then_some(1),
        id: item["id"].as_str()?.into(),
        author: item.pointer("/authorDetails/displayName")?.as_str()?.into(),
        viewer_id: item
            .pointer("/authorDetails/channelId")
            .and_then(Value::as_str)
            .map(str::to_owned),
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
    if c.live_platform != "youtube" || c.youtube_video.is_empty() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Inner;

    #[test]
    fn skips_english_usernames_only_in_chinese_mode() {
        assert!(has_english("Mike"));
        assert!(has_english("小柔fans"));
        assert!(!has_english("小明123"));
        assert_eq!(sanitize_author("小柔fans", false), None);
        assert_eq!(sanitize_author("小明", false).as_deref(), Some("小明"));
        assert_eq!(sanitize_author("Mike", true).as_deref(), Some("Mike"));
    }

    #[test]
    fn accepts_unified_douyin_tiktok_and_legacy_events() {
        let dir = tempfile::tempdir().unwrap();
        let app = Inner::new(dir.path().to_path_buf()).unwrap();

        let join_en = Incoming {
            id: "j1".into(),
            author: "Mike_99".into(),
            viewer_id: None,
            text: String::new(),
            gift: false,
            kind: Some("join".into()),
            platform: Some("douyin".into()),
            gift_name: None,
            gift_count: None,
        };
        assert!(receive(&app, join_en, "local").unwrap());
        let events = lock(&app.view).events.clone();
        assert!(events.back().unwrap().text.contains("已跳过英文用户名"));

        let gift_tt = Incoming {
            id: "g1".into(),
            author: "Alice".into(),
            viewer_id: None,
            text: String::new(),
            gift: false,
            kind: Some("gift".into()),
            platform: Some("tiktok".into()),
            gift_name: Some("Rose".into()),
            gift_count: Some(5),
        };
        assert!(receive(&app, gift_tt, "local").unwrap());
        let events = lock(&app.view).events.clone();
        assert_eq!(events.back().unwrap().kind, "TikTok·礼物");
        assert!(events.back().unwrap().text.contains("Rose×5"));

        // Legacy payload without kind/platform still works
        let legacy: Incoming =
            serde_json::from_str(r#"{"id":"l1","author":"小明","text":"晚上好","gift":false}"#)
                .unwrap();
        assert!(receive(&app, legacy, "local").unwrap());
        // Duplicate id is ignored
        let dup: Incoming =
            serde_json::from_str(r#"{"id":"l1","author":"小明","text":"晚上好","gift":false}"#)
                .unwrap();
        assert!(!receive(&app, dup, "local").unwrap());
    }

    #[test]
    fn buffers_gifts_while_singing_and_does_not_interrupt_with_joins() {
        use crate::session::Run;
        use std::sync::{
            Arc, Mutex,
            atomic::{AtomicBool, AtomicU64, AtomicUsize},
        };
        use tokio::sync::{mpsc, watch};

        let dir = tempfile::tempdir().unwrap();
        let app = Inner::new(dir.path().to_path_buf()).unwrap();
        let (wire, _wrx) = mpsc::channel(8);
        let (jobs, mut jrx) = mpsc::channel(8);
        let (gifts, mut grx) = mpsc::channel(8);
        let (epoch, _) = watch::channel(0);
        let (choreo, _) = watch::channel(false);
        let run = Arc::new(Run {
            stop: CancellationToken::new(),
            epoch,
            wire,
            jobs,
            gifts,
            choreo,
            next_action: AtomicU64::new(0),
            history: Default::default(),
            variants: Default::default(),
            singing: AtomicBool::new(true),
            busy: AtomicBool::new(true),
            last_activity_ms: AtomicU64::new(0),
            last_song_end_ms: AtomicU64::new(0),
            last_ambient_ms: AtomicU64::new(0),
            last_join_welcome_ms: AtomicU64::new(0),
            song_gifts: Mutex::new(Vec::new()),
            next_track_index: AtomicUsize::new(0),
        });
        *lock(&app.run) = Some(run.clone());
        app.phase("live", "ready");

        // While singing: join does not enqueue, gifts go to song_gifts buffer
        receive(
            &app,
            Incoming {
                id: "j-sing".into(),
                author: "小美".into(),
                viewer_id: None,
                text: String::new(),
                gift: false,
                kind: Some("join".into()),
                platform: Some("douyin".into()),
                gift_name: None,
                gift_count: None,
            },
            "local",
        )
        .unwrap();
        assert!(jrx.try_recv().is_err());

        for (id, author, name, count) in [
            ("g-1", "小明", "玫瑰", 2),
            ("g-2", "小明", "玫瑰", 3),
            ("g-3", "Bob88", "火箭", 1),
        ] {
            receive(
                &app,
                Incoming {
                    id: id.into(),
                    author: author.into(),
                    viewer_id: None,
                    text: String::new(),
                    gift: true,
                    kind: Some("gift".into()),
                    platform: Some("douyin".into()),
                    gift_name: Some(name.into()),
                    gift_count: Some(count),
                },
                "local",
            )
            .unwrap();
        }
        // Not sent to live gift speech queue yet!
        assert!(grx.try_recv().is_err());
        let buffered = lock(&run.song_gifts).clone();
        assert_eq!(buffered.len(), 2);
        assert_eq!(buffered[0].author.as_deref(), Some("小明"));
        assert_eq!(buffered[0].count, 5);
        assert_eq!(buffered[1].author, None); // Bob88 redacted in zh mode
        assert_eq!(buffered[1].gift_name, "火箭");

        // When idle (not singing, not busy): Chinese join enqueues welcome, English join skipped
        run.singing.store(false, Ordering::SeqCst);
        run.busy.store(false, Ordering::SeqCst);
        receive(
            &app,
            Incoming {
                id: "j-en-idle".into(),
                author: "Kevin".into(),
                viewer_id: None,
                text: String::new(),
                gift: false,
                kind: Some("join".into()),
                platform: Some("douyin".into()),
                gift_name: None,
                gift_count: None,
            },
            "local",
        )
        .unwrap();
        assert!(jrx.try_recv().is_err());

        receive(
            &app,
            Incoming {
                id: "j-zh-idle".into(),
                author: "阿花".into(),
                viewer_id: None,
                text: String::new(),
                gift: false,
                kind: Some("join".into()),
                platform: Some("douyin".into()),
                gift_name: None,
                gift_count: None,
            },
            "local",
        )
        .unwrap();
        assert!(jrx.try_recv().is_ok());

        // Second join immediately after is skipped by cooldown
        receive(
            &app,
            Incoming {
                id: "j-zh-cd".into(),
                author: "阿强".into(),
                viewer_id: None,
                text: String::new(),
                gift: false,
                kind: Some("join".into()),
                platform: Some("douyin".into()),
                gift_name: None,
                gift_count: None,
            },
            "local",
        )
        .unwrap();
        assert!(jrx.try_recv().is_err());
    }
}
