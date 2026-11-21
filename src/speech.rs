use crate::{
    audio,
    config::now,
    dialogue::{self, PromptKind},
    session::Run,
    state::{App, lock},
    wire::{self, Command, Packet},
};
use anyhow::{Context, Result, ensure};
use bytes::Bytes;
use serde_json::json;
use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use tokio::{sync::mpsc, time::Instant};

pub enum Job {
    Speak(String),
    Reply(String),
    AutoReply {
        kind: PromptKind,
        prompt: String,
        label: String,
    },
    Media(Bytes, String),
    Track(uuid::Uuid),
}
pub struct Offer {
    pub epoch: u64,
    pub at: u64,
    pub job: Job,
}

pub fn enqueue(app: &App, job: Job) -> Result<()> {
    let run = app.active()?;
    enqueue_in(app, &run, job, false)
}
pub fn enqueue_in(app: &App, run: &Run, job: Job, priority: bool) -> Result<()> {
    if matches!(&job, Job::Speak(_) | Job::Reply(_) | Job::AutoReply { .. }) {
        ensure!(
            app.config().tts_mode != "system",
            "系统音色用于本机试听。实播请选择 MLX、授权语音服务或录音"
        );
    }
    let queue = if priority { &run.gifts } else { &run.jobs };
    let permit = queue.try_reserve().context("播出队列已满或已关闭")?;
    let epoch = *run.epoch.borrow();
    {
        lock(&app.view).pending += 1;
    }
    permit.send(Offer {
        epoch,
        at: now(),
        job,
    });
    Ok(())
}
pub fn interrupt(app: &App, run: &Run) {
    run.epoch.send_modify(|g| *g = g.wrapping_add(1));
    let mut v = lock(&app.view);
    v.subtitle.clear();
    v.subtitle_until = 0;
    v.metrics.interrupts += 1;
}
async fn send(run: &Run, epoch: u64, cmd: Command) -> Result<()> {
    ensure!(epoch == *run.epoch.borrow(), "播出内容已取消");
    run.wire
        .send(Packet { epoch, cmd })
        .await
        .context("信令已断开")
}
async fn stream_pcm(
    app: &App,
    run: &Run,
    epoch: u64,
    at: &mut Option<u64>,
    pcm: Bytes,
    text: String,
) -> Result<()> {
    send(run, epoch, Command::Line(text, (pcm.len() / 48) as u64)).await?;
    let mut deadline = Instant::now();
    for start in (0..pcm.len()).step_by(audio::FRAME) {
        tokio::time::sleep_until(deadline).await;
        let end = (start + audio::FRAME).min(pcm.len());
        send(run, epoch, Command::Audio(pcm.slice(start..end))).await?;
        if let Some(queued) = at.take() {
            lock(&app.view).metrics.last_first_audio_ms = Some(now().saturating_sub(queued));
        }
        // Avoid a burst of overdue frames after the OS wakes or the socket is congested.
        deadline = std::cmp::max(deadline + Duration::from_millis(20), Instant::now());
    }
    tokio::time::sleep_until(deadline).await;
    Ok(())
}
async fn reply(app: &App, run: &Run, o: u64, at: &mut Option<u64>, text: &str) -> Result<()> {
    let c = app.config();
    let (tx, mut rx) = mpsc::channel(6);
    let history = lock(&run.history).iter().cloned().collect::<Vec<_>>();
    let produce = dialogue::sentences(&app.http, &c, PromptKind::Chat, text, &history, tx);
    let mut answer = String::new();
    let play = async {
        while let Some(line) = rx.recv().await {
            answer.push_str(&line);
            let pcm = audio::synth(&app.http, &c, &line).await?;
            stream_pcm(app, run, o, at, pcm, line).await?;
        }
        Ok::<_, anyhow::Error>(())
    };
    tokio::try_join!(produce, play)?;
    let mut history = lock(&run.history);
    if history.len() >= 6 {
        history.pop_front();
    }
    history.push_back((text.into(), answer));
    Ok(())
}
async fn auto_reply(
    app: &App,
    run: &Run,
    o: u64,
    at: &mut Option<u64>,
    kind: PromptKind,
    prompt: &str,
    label: &str,
) -> Result<()> {
    let c = app.config();
    let (tx, mut rx) = mpsc::channel(6);
    let produce = dialogue::sentences(&app.http, &c, kind, prompt, &[], tx);
    let mut answer = String::new();
    let play = async {
        while let Some(line) = rx.recv().await {
            answer.push_str(&line);
            let pcm = audio::synth(&app.http, &c, &line).await?;
            stream_pcm(app, run, o, at, pcm, line).await?;
        }
        Ok::<_, anyhow::Error>(())
    };
    tokio::try_join!(produce, play)?;
    app.event(label, answer);
    Ok(())
}
struct AbortOnDrop(Option<tokio::task::JoinHandle<()>>);
impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        if let Some(h) = self.0.take() {
            h.abort();
        }
    }
}

fn clear_stage_song(app: &App) {
    let had_song = lock(&app.screen).song.take().is_some();
    if had_song {
        app.notify_stage(json!({"event": "song_stop"}));
    }
}

fn notify_bgm_state(app: &App) {
    let bgm = crate::output::bgm_snapshot(app);
    app.notify_stage(json!({"event": "bgm_state", "bgm": bgm}));
}

async fn play_track(app: &App, run: &Run, o: u64, at: &mut Option<u64>, id: uuid::Uuid) -> Result<()> {
    let tracks = crate::library::list(app)?;
    let (title, next_idx) = tracks
        .iter()
        .enumerate()
        .find(|(_, t)| t.id == id)
        .map(|(i, t)| (t.title.clone(), (i + 1) % tracks.len().max(1)))
        .unwrap_or_else(|| ("当前曲目".into(), 0));
    if !tracks.is_empty() {
        run.next_track_index.store(next_idx, Ordering::Relaxed);
    }
    let pcm = crate::library::audio(app, id).await?;
    let cfg = app.config();
    let use_stage_audio = cfg.song_audio_mode == "stage";

    {
        // Lock song_gifts briefly when entering singing state for strict ordering with gift buffering.
        let _g = lock(&run.song_gifts);
        run.singing.store(true, Ordering::SeqCst);
    }
    {
        let mut v = lock(&app.view);
        v.singing = true;
        v.current_song = title.clone();
    }
    // Pause BGM cleanly before singing starts.
    notify_bgm_state(app);

    let playback_id = uuid::Uuid::new_v4().to_string();
    let vol = cfg.song_volume as f32 / 100.0;
    if use_stage_audio {
        lock(&app.screen).song = Some(crate::state::StageSong {
            playback_id: playback_id.clone(),
            track_id: id.to_string(),
            title: title.clone(),
            state: "prepare".into(),
            started_at: 0,
            volume: vol,
        });
        app.notify_stage(json!({
            "event": "song_prepare",
            "playback_id": playback_id,
            "track_id": id.to_string(),
            "title": title,
            "volume": vol,
        }));
    }

    let sing_act = cfg.active_singing_action().trim();
    if !sing_act.is_empty() {
        send(run, o, Command::Signal(9, wire::text_msg(sing_act))).await?;
        app.event("唱歌", format!("开始演唱《{title}》· {sing_act}"));
        tokio::time::sleep(Duration::from_millis(400)).await;
    } else {
        app.event("唱歌", format!("开始演唱《{title}》"));
        if use_stage_audio {
            // Brief window so Stage finishes prebuffering the local Hi-Fi track.
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    }

    let sync_offset_ms = if use_stage_audio {
        app.config().song_sync_offset_ms as u64
    } else {
        0
    };
    let _sync_guard = if use_stage_audio {
        let app_clone = app.clone();
        let pid = playback_id.clone();
        let tid = id.to_string();
        let song_title = title.clone();
        Some(AbortOnDrop(Some(tokio::spawn(async move {
            if sync_offset_ms > 0 {
                tokio::time::sleep(Duration::from_millis(sync_offset_ms)).await;
            }
            let started_at = now();
            let current_vol = app_clone.config().song_volume as f32 / 100.0;
            let active = {
                let mut sc = lock(&app_clone.screen);
                if let Some(ref mut s) = sc.song
                    && s.playback_id == pid
                {
                    s.state = "play".into();
                    s.started_at = started_at;
                    s.volume = current_vol;
                    true
                } else {
                    false
                }
            };
            if active {
                app_clone.notify_stage(json!({
                    "event": "song_play",
                    "playback_id": pid,
                    "track_id": tid,
                    "title": song_title,
                    "volume": current_vol,
                }));
            }
        }))))
    } else {
        None
    };

    stream_pcm(app, run, o, at, pcm, String::new()).await?;

    // Wait the remaining sync tail so the local Hi-Fi song finishes cleanly before unmuting TRTC.
    if use_stage_audio && sync_offset_ms > 0 {
        tokio::time::sleep(Duration::from_millis(sync_offset_ms)).await;
    }
    drop(_sync_guard);
    clear_stage_song(app);

    // Song audio finished: atomically leave gift-buffering state and collect buffered gifts.
    let buffered_gifts = {
        let mut g = lock(&run.song_gifts);
        let drained = std::mem::take(&mut *g);
        run.singing.store(false, Ordering::SeqCst);
        drained
    };
    run.last_song_end_ms.store(now(), Ordering::Relaxed);

    let post_act = cfg.active_post_sing_action().trim();
    if !post_act.is_empty() {
        send(run, o, Command::Signal(9, wire::text_msg(post_act))).await?;
    } else if !sing_act.is_empty() {
        send(run, o, Command::Signal(15, json!({}))).await?;
    }

    // Keep an explicit pause between finishing the song and starting post-song thanks.
    let gap = cfg.utterance_gap as u64;
    if gap > 0 {
        tokio::time::sleep(Duration::from_secs(gap)).await;
    }

    if cfg.tts_mode != "system" {
        let prompt = crate::director::build_post_song_thanks_prompt(&cfg, &title, &buffered_gifts);
        let label = if buffered_gifts.is_empty() {
            "唱完谢幕"
        } else {
            "唱完谢礼"
        };
        let mut thanks_at = Some(now());
        auto_reply(
            app,
            run,
            o,
            &mut thanks_at,
            PromptKind::PostSongThanks,
            &prompt,
            label,
        )
        .await?;
    }
    Ok(())
}
async fn play(app: &App, run: &Run, o: Offer) -> Result<()> {
    let mut first = Some(o.at);
    match o.job {
        Job::Speak(s) => {
            let pcm = audio::synth(&app.http, &app.config(), &s).await?;
            stream_pcm(app, run, o.epoch, &mut first, pcm, s).await?;
        }
        Job::Reply(s) => reply(app, run, o.epoch, &mut first, &s).await?,
        Job::AutoReply {
            kind,
            prompt,
            label,
        } => auto_reply(app, run, o.epoch, &mut first, kind, &prompt, &label).await?,
        Job::Track(id) => play_track(app, run, o.epoch, &mut first, id).await?,
        Job::Media(pcm, s) => {
            {
                let mut v = lock(&app.view);
                v.singing = true;
                v.current_song = if s.is_empty() {
                    "播出音频".into()
                } else {
                    s.clone()
                };
            }
            notify_bgm_state(app);
            stream_pcm(app, run, o.epoch, &mut first, pcm, s).await?;
        }
    }
    lock(&app.view).metrics.utterances += 1;
    Ok(())
}
pub async fn worker(
    app: App,
    run: Arc<Run>,
    mut rx: mpsc::Receiver<Offer>,
    mut gifts: mpsc::Receiver<Offer>,
) {
    let mut gift_streak = 0u8;
    loop {
        let next = if gift_streak >= 3 && !rx.is_empty() {
            gift_streak = 0;
            rx.try_recv().ok()
        } else {
            tokio::select! {
                biased;
                _ = run.stop.cancelled() => return,
                offer = gifts.recv() => {
                    gift_streak = gift_streak.saturating_add(1);
                    offer
                }
                offer = rx.recv() => {
                    gift_streak = 0;
                    offer
                }
            }
        };
        let Some(o) = next else { return };
        {
            let mut v = lock(&app.view);
            v.pending = v.pending.saturating_sub(1);
        }
        let mut changes = run.epoch.subscribe();
        if *changes.borrow_and_update() != o.epoch {
            continue;
        }
        run.busy.store(true, Ordering::SeqCst);
        lock(&app.view).voice_busy = true;
        notify_bgm_state(&app);
        let mut interrupted = false;
        let mut stopped = false;
        tokio::select! {
            biased;
            _ = run.stop.cancelled() => {
                stopped = true;
            }
            _ = changes.changed() => {
                interrupted = true;
            }
            result = play(&app, &run, o) => {
                if let Err(e) = result {
                    app.event("播出错误", e.to_string());
                }
            }
        }
        clear_stage_song(&app);
        {
            let mut g = lock(&run.song_gifts);
            run.singing.store(false, Ordering::SeqCst);
            if interrupted || stopped {
                g.clear();
            }
        }
        {
            let mut v = lock(&app.view);
            v.voice_busy = false;
            v.singing = false;
            v.current_song.clear();
        }
        notify_bgm_state(&app);
        run.last_activity_ms.store(now(), Ordering::Relaxed);
        crate::choreo::hold(&app, &run);
        if stopped {
            run.busy.store(false, Ordering::SeqCst);
            break;
        }
        // Keep a mandatory quiet interval between consecutive utterances/behaviors.
        if !interrupted {
            let gap = app.config().utterance_gap as u64;
            if gap > 0 {
                tokio::select! {
                    biased;
                    _ = run.stop.cancelled() => {
                        run.busy.store(false, Ordering::SeqCst);
                        break;
                    }
                    _ = changes.changed() => (),
                    _ = tokio::time::sleep(Duration::from_secs(gap)) => (),
                }
            }
        }
        run.busy.store(false, Ordering::SeqCst);
    }
}
