use crate::{
    audio,
    config::{Settings, now},
    dialogue,
    prompts::{Answer, Scene, Snapshot},
    replies::{self, Chat, Decision, Event},
    session::Run,
    state::{App, ReplyMetrics, lock},
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
    Reply(Chat),
    AutoReply { event: Event, label: String },
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
        let cfg = app.config();
        if let Job::Reply(chat) = &job {
            let profile = app.prompts.active(&cfg.prompt_profile, cfg.is_english())?;
            if matches!(profile.chat_answer(&chat.text), Some(Answer::Skip)) {
                app.event("跳过回复", "按 QA 规则忽略，未播报");
                return Ok(());
            }
        }
        ensure!(
            cfg.tts_mode != "system",
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
struct ReplyPlayback<'a> {
    app: &'a App,
    run: &'a Run,
    cfg: &'a Settings,
    epoch: u64,
    at: &'a mut Option<u64>,
}

impl ReplyPlayback<'_> {
    async fn speak(&mut self, line: String) -> Result<u64> {
        let start = Instant::now();
        let pcm = audio::synth(&self.app.http, self.cfg, &line).await?;
        let tts_ms = dialogue::elapsed(start);
        stream_pcm(self.app, self.run, self.epoch, self.at, pcm, line).await?;
        Ok(tts_ms)
    }

    async fn response(
        &mut self,
        profile: &Snapshot,
        decision: Decision,
        history: &[(String, String)],
    ) -> Result<Option<String>> {
        let queued = *self.at;
        let mut timing = ReplyMetrics {
            queue_ms: queued.map(|at| now().saturating_sub(at)),
            ..Default::default()
        };
        let answer = match decision {
            Decision::Skip => {
                timing.source = "skip";
                None
            }
            Decision::Speak(line) => {
                timing.source = "template";
                timing.tts_ms = Some(self.speak(line.clone()).await?);
                Some(line)
            }
            Decision::Generate(input) => {
                timing.source = "model";
                self.generate(profile, &input, history, &mut timing).await?
            }
        };
        let mut view = lock(&self.app.view);
        if queued.is_some() && self.at.is_none() {
            timing.first_audio_ms = view.metrics.last_first_audio_ms;
        }
        view.metrics.last_reply = Some(timing);
        Ok(answer)
    }

    async fn generate(
        &mut self,
        profile: &Snapshot,
        input: &str,
        history: &[(String, String)],
        timing: &mut ReplyMetrics,
    ) -> Result<Option<String>> {
        let (tx, mut rx) = mpsc::channel(6);
        let app = self.app;
        let cfg = self.cfg;
        let generate = dialogue::sentences(&app.http, cfg, &profile.system, input, history, tx);
        let mut answer = String::new();
        let mut skipped = false;
        let play = async {
            while let Some(line) = rx.recv().await {
                if answer.is_empty() && dialogue::is_skip_reply(&line) {
                    skipped = true;
                }
                if skipped {
                    continue;
                }
                answer.push_str(&line);
                let tts_ms = self.speak(line).await?;
                timing.tts_ms.get_or_insert(tts_ms);
            }
            Ok::<_, anyhow::Error>(())
        };
        (timing.generation, _) = tokio::try_join!(generate, play)?;
        Ok((!answer.is_empty()).then_some(answer))
    }
}

async fn reply(app: &App, run: &Run, epoch: u64, at: &mut Option<u64>, chat: Chat) -> Result<bool> {
    let cfg = app.config();
    let profile = app.prompts.active(&cfg.prompt_profile, cfg.is_english())?;
    let decision = lock(&run.variants).chat(&profile, &chat, cfg.is_english());
    let history = lock(&run.history).for_chat(&chat, &profile.version);
    let mut playback = ReplyPlayback {
        app,
        run,
        cfg: &cfg,
        epoch,
        at,
    };
    if let Some(answer) = playback.response(&profile, decision, &history).await? {
        lock(&run.history).remember(
            &chat,
            &profile.version,
            chat.model_input(cfg.is_english()),
            answer,
        );
        Ok(true)
    } else {
        app.event("跳过回复", "按 QA 规则忽略，未播报");
        Ok(false)
    }
}

async fn auto_reply(
    app: &App,
    run: &Run,
    epoch: u64,
    at: &mut Option<u64>,
    event: Event,
    label: &str,
) -> Result<bool> {
    let cfg = app.config();
    let profile = app.prompts.active(&cfg.prompt_profile, cfg.is_english())?;
    let decision = lock(&run.variants).event(&profile, event);
    let mut playback = ReplyPlayback {
        app,
        run,
        cfg: &cfg,
        epoch,
        at,
    };
    if let Some(answer) = playback.response(&profile, decision, &[]).await? {
        app.event(label, answer);
        Ok(true)
    } else {
        app.event("跳过播报", label);
        Ok(false)
    }
}

pub async fn preview_reply(app: &App, text: String) -> Result<Bytes> {
    let cfg = app.config();
    let profile = app.prompts.active(&cfg.prompt_profile, cfg.is_english())?;
    let (audio, timing) =
        match replies::Variants::default().chat(&profile, &text.into(), cfg.is_english()) {
            Decision::Skip => (
                Bytes::new(),
                ReplyMetrics {
                    source: "skip",
                    ..Default::default()
                },
            ),
            Decision::Speak(line) => {
                let start = Instant::now();
                let pcm = audio::synth(&app.http, &cfg, &line).await?;
                let timing = ReplyMetrics {
                    source: "template",
                    tts_ms: Some(dialogue::elapsed(start)),
                    ..Default::default()
                };
                (pcm, timing)
            }
            Decision::Generate(text) => preview_generated(app, &cfg, &profile, &text).await?,
        };
    if lock(&app.run).is_none() {
        lock(&app.view).metrics.last_reply = Some(timing);
    }
    ensure!(!audio.is_empty(), "按 QA 规则该问题无需回复");
    Ok(audio)
}

async fn preview_generated(
    app: &App,
    cfg: &Settings,
    profile: &Snapshot,
    text: &str,
) -> Result<(Bytes, ReplyMetrics)> {
    let (tx, mut rx) = mpsc::channel(8);
    let generate = dialogue::sentences(&app.http, cfg, &profile.system, text, &[], tx);
    let speak = async {
        let mut pcm = Vec::new();
        let mut skipped = false;
        let mut tts_ms = None;
        while let Some(line) = rx.recv().await {
            if pcm.is_empty() && dialogue::is_skip_reply(&line) {
                skipped = true;
            }
            if !skipped {
                let start = Instant::now();
                let part = audio::synth(&app.http, cfg, &line).await?;
                tts_ms.get_or_insert_with(|| dialogue::elapsed(start));
                ensure!(pcm.len() + part.len() < 32_000_000, "回复语音过长");
                pcm.extend_from_slice(&part);
            }
        }
        Ok::<_, anyhow::Error>((Bytes::from(pcm), tts_ms))
    };
    let (generation, (pcm, tts_ms)) = tokio::try_join!(generate, speak)?;
    Ok((
        pcm,
        ReplyMetrics {
            source: "model",
            tts_ms,
            generation,
            ..Default::default()
        },
    ))
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

fn song_phrase(
    app: &App,
    run: &Run,
    cfg: &Settings,
    scene: Scene,
    fields: &[(&str, &str)],
) -> Option<(Arc<Snapshot>, String)> {
    let profile = app
        .prompts
        .active(&cfg.prompt_profile, cfg.is_english())
        .ok()?;
    let line = lock(&run.variants).scene(&profile, scene, fields)?;
    Some((profile, line))
}

async fn play_track(
    app: &App,
    run: &Run,
    o: u64,
    at: &mut Option<u64>,
    id: uuid::Uuid,
) -> Result<()> {
    let tracks = crate::library::list(app)?;
    let (raw_title, next_idx) = tracks
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
    let title = crate::config::resolve_song_title(&cfg, &raw_title);
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
    // Pause BGM cleanly before the spoken intro and singing start.
    notify_bgm_state(app);

    let playback_id = uuid::Uuid::new_v4().to_string();
    let vol = cfg.song_volume as f32 / 100.0;
    if use_stage_audio {
        // Prebuffer the Hi-Fi stereo track while she speaks the intro (TRTC audio stays unmuted until song_play).
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

    let mut spoke_intro = false;
    if cfg.tts_mode != "system"
        && let Some((_, intro)) = song_phrase(app, run, &cfg, Scene::SongIntro, &[("song", &title)])
    {
        match audio::synth(&app.http, &cfg, &intro).await {
            Ok(intro_pcm) => {
                app.event("开唱报幕", intro.clone());
                stream_pcm(app, run, o, at, intro_pcm, intro).await?;
                spoke_intro = true;
                // Give Vidu a clean beat to finish rendering the spoken intro before starting to sing.
                tokio::time::sleep(Duration::from_millis(900)).await;
            }
            Err(e) => {
                ensure!(o == *run.epoch.borrow(), "播出内容已取消");
                app.event("开唱报幕跳过", format!("语音合成失败，直接开始演唱：{e}"));
            }
        }
    }

    let sing_act = cfg.active_singing_action().trim();
    if !sing_act.is_empty() {
        send(run, o, Command::Signal(9, wire::text_msg(sing_act))).await?;
        app.event("唱歌", format!("开始演唱《{title}》· {sing_act}"));
        tokio::time::sleep(Duration::from_millis(400)).await;
    } else {
        app.event("唱歌", format!("开始演唱《{title}》"));
        if use_stage_audio && !spoke_intro {
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
        let (scene, label) = if buffered_gifts.is_empty() {
            (Scene::SongEnd, "唱完谢幕")
        } else {
            (Scene::SongGifts, "唱完谢礼")
        };
        let gifts = crate::director::format_gift_summary(&cfg, &buffered_gifts);
        if let Some((profile, line)) = song_phrase(
            app,
            run,
            &cfg,
            scene,
            &[("song", &title), ("gifts", &gifts)],
        ) {
            let mut thanks_at = Some(now());
            let mut playback = ReplyPlayback {
                app,
                run,
                cfg: &cfg,
                epoch: o,
                at: &mut thanks_at,
            };
            playback
                .response(&profile, Decision::Speak(line.clone()), &[])
                .await?;
            app.event(label, line);
        }
    }
    Ok(())
}
async fn play(app: &App, run: &Run, o: Offer) -> Result<bool> {
    let mut first = Some(o.at);
    let mut spoken = true;
    match o.job {
        Job::Speak(s) => {
            let pcm = audio::synth(&app.http, &app.config(), &s).await?;
            stream_pcm(app, run, o.epoch, &mut first, pcm, s).await?;
        }
        Job::Reply(chat) => spoken = reply(app, run, o.epoch, &mut first, chat).await?,
        Job::AutoReply { event, label } => {
            spoken = auto_reply(app, run, o.epoch, &mut first, event, &label).await?
        }
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
    if spoken {
        lock(&app.view).metrics.utterances += 1;
    }
    Ok(spoken)
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
        let mut skipped = false;
        tokio::select! {
            biased;
            _ = run.stop.cancelled() => {
                stopped = true;
            }
            _ = changes.changed() => {
                interrupted = true;
            }
            result = play(&app, &run, o) => {
                match result {
                    Ok(false) => skipped = true,
                    Ok(true) => (),
                    Err(e) => app.event("播出错误", e.to_string()),
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
        if !skipped {
            run.last_activity_ms.store(now(), Ordering::Relaxed);
            crate::choreo::hold(&app, &run);
        }
        if stopped {
            run.busy.store(false, Ordering::SeqCst);
            break;
        }
        // Keep a mandatory quiet interval between consecutive utterances/behaviors.
        if !interrupted && !skipped {
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
