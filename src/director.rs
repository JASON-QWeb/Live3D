use crate::{
    config::{Settings, now},
    dialogue::PromptKind,
    session::{PendingGift, Run},
    speech::{self, Job},
    state::{App, lock},
};
use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

pub fn build_ambient_welcome_prompt(cfg: &Settings, phrase: &str) -> String {
    if cfg.is_english() {
        format!(
            "[System event: periodic welcome between songs] Say one short, warm welcome line in natural English inspired by \"{phrase}\". Output only the spoken text."
        )
    } else {
        format!(
            "[系统事件：不唱歌时的定时暖场欢迎] 现在没有在唱歌，请参考句式「{phrase}」，用自然亲切的中文说出简短的1句欢迎语，直接输出要朗读的正文。"
        )
    }
}

pub fn build_join_welcome_prompt(cfg: &Settings, author: &str) -> String {
    if cfg.is_english() {
        format!(
            "[System event: viewer joined] Viewer \"{author}\" just entered the livestream. Greet \"{author}\" warmly by name in one short, natural English sentence. Output only the spoken text."
        )
    } else {
        format!(
            "[系统事件：新观众进入直播间] 观众「{author}」刚刚进入直播间。请自然亲切地用简短1句话欢迎对方，直接叫出「{author}」的名字，直接输出要朗读的正文。"
        )
    }
}

pub fn build_gift_thanks_prompt(
    cfg: &Settings,
    author: Option<&str>,
    gift_desc: &str,
    extra_text: &str,
) -> String {
    let extra = extra_text.trim();
    if cfg.is_english() {
        let who = author.unwrap_or("a kind viewer");
        if !extra.is_empty() && extra != gift_desc {
            format!(
                "[System event: gift received] {who} sent {gift_desc} with message: \"{extra}\". Thank them warmly in 1 to 2 short natural English sentences. Output only the spoken text."
            )
        } else {
            format!(
                "[System event: gift received] {who} sent {gift_desc}. Thank them warmly in 1 to 2 short natural English sentences. Output only the spoken text."
            )
        }
    } else {
        match author {
            Some(name) => {
                if !extra.is_empty() && extra != gift_desc {
                    format!(
                        "[系统事件：收到观众礼物] 观众「{name}」送了「{gift_desc}」，并留言：「{extra}」。请用1到2句自然真诚的中文感谢「{name}」送的礼物，直接输出要朗读的正文。"
                    )
                } else {
                    format!(
                        "[系统事件：收到观众礼物] 观众「{name}」送了「{gift_desc}」。请用1到2句自然真诚的中文感谢「{name}」送的礼物，直接输出要朗读的正文。"
                    )
                }
            }
            None => format!(
                "[系统事件：收到观众礼物] 一位热心老板送了「{gift_desc}」（请称呼对方为“老板”或“这位朋友”，不要念出或编造任何英文用户名）。请用1到2句自然真诚的中文感谢这份礼物，直接输出要朗读的正文。"
            ),
        }
    }
}

pub fn format_gift_summary(cfg: &Settings, gifts: &[PendingGift]) -> String {
    let cap = 8;
    let mut items = Vec::new();
    for g in gifts.iter().take(cap) {
        let qty = if g.count > 1 {
            if cfg.is_english() {
                format!("{} x{}", g.gift_name, g.count)
            } else {
                format!("{}×{}", g.gift_name, g.count)
            }
        } else {
            g.gift_name.clone()
        };
        let entry = match &g.author {
            Some(name) => {
                if cfg.is_english() {
                    format!("{name} ({qty})")
                } else {
                    format!("「{name}」送的{qty}")
                }
            }
            None => {
                if cfg.is_english() {
                    format!("a kind viewer ({qty})")
                } else {
                    format!("一位热心老板送的{qty}")
                }
            }
        };
        items.push(entry);
    }
    if gifts.len() > cap {
        if cfg.is_english() {
            items.push(format!("and {} more gift senders", gifts.len() - cap));
        } else {
            items.push(format!("以及其他{}位送礼物的朋友", gifts.len() - cap));
        }
    }
    if cfg.is_english() {
        items.join(", ")
    } else {
        items.join("、")
    }
}

pub fn build_post_song_thanks_prompt(
    cfg: &Settings,
    song_title: &str,
    gifts: &[PendingGift],
) -> String {
    if gifts.is_empty() {
        if cfg.is_english() {
            format!(
                "[System event: finished singing \"{song_title}\"] You just finished singing. Thank everyone warmly for listening in 1 to 2 short natural English sentences. Output only the spoken text."
            )
        } else {
            format!(
                "[系统事件：刚唱完歌曲《{song_title}》] 请自然亲切地用1到2句中文谢谢大家听我唱歌，直接输出要朗读的正文。"
            )
        }
    } else {
        let summary = format_gift_summary(cfg, gifts);
        let has_redacted = gifts.iter().any(|g| g.author.is_none());
        if cfg.is_english() {
            format!(
                "[System event: finished singing \"{song_title}\"] You just finished singing. During the song, you received these gifts: {summary}. First thank everyone for listening, then warmly thank the gift senders in 2 to 3 short natural English sentences. Output only the spoken text."
            )
        } else if has_redacted {
            format!(
                "[系统事件：刚唱完歌曲《{song_title}》] 唱歌期间收到了以下礼物：{summary}。请先谢谢大家听歌，再真诚感谢送礼物的观众（对于未写名字的热心老板请称呼“老板”或“朋友”，不要念出任何英文名），控制在2到3句，直接输出要朗读的正文。"
            )
        } else {
            format!(
                "[系统事件：刚唱完歌曲《{song_title}》] 唱歌期间收到了以下礼物：{summary}。请先谢谢大家听歌，再真诚感谢送礼物的观众，控制在2到3句，直接输出要朗读的正文。"
            )
        }
    }
}

pub fn try_buffer_song_gift(
    run: &Run,
    author: Option<String>,
    gift_name: String,
    count: u32,
) -> bool {
    let mut list = lock(&run.song_gifts);
    if !run.singing.load(Ordering::SeqCst) {
        return false;
    }
    let add = count.max(1);
    if let Some(existing) = list
        .iter_mut()
        .find(|g| g.author == author && g.gift_name == gift_name)
    {
        existing.count = existing.count.saturating_add(add);
    } else if list.len() < 100 {
        list.push(PendingGift {
            author,
            gift_name,
            count: add,
        });
    }
    true
}

pub async fn worker(app: App, run: Arc<Run>) {
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            biased;
            _ = run.stop.cancelled() => break,
            _ = tick.tick() => {
                let (live, pending) = {
                    let v = lock(&app.view);
                    (v.phase == "live", v.pending)
                };
                if !live
                    || pending > 0
                    || run.singing.load(Ordering::Relaxed)
                    || run.busy.load(Ordering::Relaxed)
                {
                    continue;
                }
                let cfg = app.config();
                let current = now();
                let gap_ms = cfg.utterance_gap as u64 * 1000;
                let last_act = run.last_activity_ms.load(Ordering::Relaxed);
                if last_act == 0 || current.saturating_sub(last_act) < gap_ms {
                    continue;
                }

                // Priority 1: Scheduled sequential auto-singing
                if cfg.auto_sing {
                    let last_song = run.last_song_end_ms.load(Ordering::Relaxed);
                    let sing_gap_ms = cfg.auto_sing_gap as u64 * 1000;
                    if last_song > 0 && current.saturating_sub(last_song) >= sing_gap_ms {
                        if let Ok(tracks) = crate::library::list(&app)
                            && !tracks.is_empty()
                        {
                            let idx =
                                run.next_track_index.load(Ordering::Relaxed) % tracks.len();
                            let track = &tracks[idx];
                            run.last_song_end_ms.store(current, Ordering::Relaxed);
                            if speech::enqueue_in(&app, &run, Job::Track(track.id), false).is_ok() {
                                app.event(
                                    "自动唱歌",
                                    format!("定时顺序排入曲目《{}》", track.title),
                                );
                                continue;
                            }
                        }
                    }
                }

                // Priority 2: Ambient welcome while not singing
                if cfg.auto_welcome && cfg.tts_mode != "system" {
                    let phrases = cfg.active_welcome_phrases();
                    if !phrases.is_empty() {
                        let welcome_gap_ms = cfg.ambient_welcome_gap as u64 * 1000;
                        let last_amb = run.last_ambient_ms.load(Ordering::Relaxed);
                        if current.saturating_sub(last_act) >= welcome_gap_ms
                            && current.saturating_sub(last_amb) >= welcome_gap_ms
                        {
                            let idx = run.next_welcome_index.fetch_add(1, Ordering::Relaxed)
                                % phrases.len();
                            let phrase = &phrases[idx];
                            let prompt = build_ambient_welcome_prompt(&cfg, phrase);
                            run.last_ambient_ms.store(current, Ordering::Relaxed);
                            let _ = speech::enqueue_in(
                                &app,
                                &run,
                                Job::AutoReply {
                                    kind: PromptKind::AmbientWelcome,
                                    prompt,
                                    label: format!("定时欢迎 · {phrase}"),
                                },
                                false,
                            );
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn post_song_prompt_merges_and_redacts_english_senders_in_zh() {
        let cfg = Settings::default();
        let gifts = vec![
            PendingGift {
                author: Some("小明".into()),
                gift_name: "玫瑰".into(),
                count: 3,
            },
            PendingGift {
                author: None,
                gift_name: "火箭".into(),
                count: 1,
            },
        ];
        let prompt = build_post_song_thanks_prompt(&cfg, "夜空中最亮的星", &gifts);
        assert!(prompt.contains("夜空中最亮的星"));
        assert!(prompt.contains("「小明」送的玫瑰×3"));
        assert!(prompt.contains("一位热心老板送的火箭"));
        assert!(prompt.contains("不要念出任何英文名"));
    }
}
