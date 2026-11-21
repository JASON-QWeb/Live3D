use crate::{
    config::{Settings, now},
    prompts::Scene,
    replies::Event,
    session::{PendingGift, Run},
    speech::{self, Job},
    state::{App, lock},
};
use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

pub fn format_gift_summary(cfg: &Settings, gifts: &[PendingGift]) -> String {
    let cap = 8;
    let mut items = Vec::new();
    for g in gifts.iter().take(cap) {
        let qty = if g.count > 1 {
            if cfg.is_english() {
                format!("{} {}", g.count, g.gift_name)
            } else {
                format!("{}个{}", g.count, g.gift_name)
            }
        } else {
            g.gift_name.clone()
        };
        let entry = match &g.author {
            Some(name) => {
                if cfg.is_english() {
                    format!("{name} for {qty}")
                } else {
                    format!("{name}送的{qty}")
                }
            }
            None => {
                if cfg.is_english() {
                    format!("a kind viewer for {qty}")
                } else {
                    format!("热心老板送的{qty}")
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
                                let clean = crate::config::resolve_song_title(&cfg, &track.title);
                                app.event(
                                    "自动唱歌",
                                    format!("定时顺序排入曲目《{clean}》"),
                                );
                                continue;
                            }
                        }
                    }
                }

                // Priority 2: Ambient welcome while not singing
                if cfg.auto_welcome && cfg.tts_mode != "system" {
                    if let Ok(profile) = app.prompts.active(&cfg.prompt_profile, cfg.is_english())
                        && !profile.lines(Scene::Ambient).is_empty()
                    {
                        let welcome_gap_ms = cfg.ambient_welcome_gap as u64 * 1000;
                        let last_amb = run.last_ambient_ms.load(Ordering::Relaxed);
                        if current.saturating_sub(last_act) >= welcome_gap_ms
                            && current.saturating_sub(last_amb) >= welcome_gap_ms
                        {
                            run.last_ambient_ms.store(current, Ordering::Relaxed);
                            let _ = speech::enqueue_in(
                                &app,
                                &run,
                                Job::AutoReply {
                                    event: Event::Ambient,
                                    label: "定时欢迎".into(),
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
    fn song_gift_summary_uses_real_names_and_counts() {
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
        let summary = format_gift_summary(&cfg, &gifts);
        assert!(summary.contains("小明送的3个玫瑰"));
        assert!(summary.contains("热心老板送的火箭"));
        assert!(!summary.contains('×'));
        assert!(!summary.contains('「'));
    }

    #[test]
    fn song_intro_uses_clean_title_and_varies_consecutive_calls() {
        let mut profile = crate::prompts::Profile::defaults(false);
        profile.retain_resolved_defaults("");
        let profile = profile.compile("", false, String::new()).unwrap();
        let mut variants = crate::replies::Variants::default();
        let fields = [("song", "云烟成雨")];
        let a = variants.scene(&profile, Scene::SongIntro, &fields).unwrap();
        let b = variants.scene(&profile, Scene::SongIntro, &fields).unwrap();
        assert!(a.contains("云烟成雨"));
        assert!(b.contains("云烟成雨"));
        assert_ne!(a, b);
    }
}
