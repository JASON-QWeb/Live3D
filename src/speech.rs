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
const INTRO_SETTLE_MS: u64 = 1000;
const SING_CUE_MIN_DELAY_MS: u64 = 800;
const MAX_LEADING_SILENCE_MS: u64 = 10_000;
const AUDIBLE_MEAN_SQ_THRESHOLD: u64 = 300 * 300; // ~ -41 dBFS for 16-bit PCM
const AUDIBLE_CONSECUTIVE_FRAMES: usize = 3; // 60 ms continuous sound

struct DelayedCue<'a> {
    delay_ms: u64,
    label: &'static str,
    action: &'a str,
}

fn leading_silence_ms(pcm: &[u8]) -> u64 {
    let max_frames = (MAX_LEADING_SILENCE_MS / 20) as usize;
    let mut audible_streak = 0usize;
    let (frames, _) = pcm.as_chunks::<{ audio::FRAME }>();
    for (idx, frame) in frames.iter().take(max_frames).enumerate() {
        let (pairs, _) = frame.as_chunks::<2>();
        let samples = pairs.len() as u64;
        let sum_sq: u64 = pairs
            .iter()
            .map(|&b| {
                let s = i16::from_le_bytes(b) as i64;
                (s * s) as u64
            })
            .sum();
        if sum_sq / samples >= AUDIBLE_MEAN_SQ_THRESHOLD {
            audible_streak += 1;
            if audible_streak >= AUDIBLE_CONSECUTIVE_FRAMES {
                let onset_frame = idx + 1 - AUDIBLE_CONSECUTIVE_FRAMES;
                return (onset_frame as u64) * 20;
            }
        } else {
            audible_streak = 0;
        }
    }
    0
}

fn singing_cue_delay_ms(pcm: &[u8], use_stage_audio: bool, sync_offset_ms: u64) -> u64 {
    let base_ms = if use_stage_audio {
        sync_offset_ms.max(SING_CUE_MIN_DELAY_MS)
    } else {
        SING_CUE_MIN_DELAY_MS
    };
    base_ms.saturating_add(leading_silence_ms(pcm))
}

async fn stream_pcm_cued(
    app: &App,
    run: &Run,
    epoch: u64,
    at: &mut Option<u64>,
    pcm: Bytes,
    text: String,
    mut cue: Option<DelayedCue<'_>>,
) -> Result<bool> {
    send(run, epoch, Command::Line(text, (pcm.len() / 48) as u64)).await?;
    let mut deadline = Instant::now();
    let mut cue_sent = false;
    for (frame_idx, start) in (0..pcm.len()).step_by(audio::FRAME).enumerate() {
        tokio::time::sleep_until(deadline).await;
        let end = (start + audio::FRAME).min(pcm.len());
        send(run, epoch, Command::Audio(pcm.slice(start..end))).await?;
        if let Some(queued) = at.take() {
            lock(&app.view).metrics.last_first_audio_ms = Some(now().saturating_sub(queued));
        }
        if let Some(pending) = cue.as_ref()
            && (frame_idx as u64) * 20 >= pending.delay_ms
        {
            send(
                run,
                epoch,
                Command::Signal(9, wire::text_msg(pending.action)),
            )
            .await?;
            app.event(pending.label, pending.action.to_owned());
            cue_sent = true;
            cue = None;
        }
        // Avoid a burst of overdue frames after the OS wakes or the socket is congested.
        deadline = std::cmp::max(deadline + Duration::from_millis(20), Instant::now());
    }
    tokio::time::sleep_until(deadline).await;
    Ok(cue_sent)
}

async fn stream_pcm(
    app: &App,
    run: &Run,
    epoch: u64,
    at: &mut Option<u64>,
    pcm: Bytes,
    text: String,
) -> Result<()> {
    stream_pcm_cued(app, run, epoch, at, pcm, text, None).await?;
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

mod song;
use song::{clear_stage_song, notify_bgm_state, play_track};
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
async fn next_offer(
    run: &Run,
    gifts: &mut mpsc::Receiver<Offer>,
    rx: &mut mpsc::Receiver<Offer>,
    gift_streak: &mut u8,
) -> Option<Offer> {
    if *gift_streak >= 3 && !rx.is_empty() {
        *gift_streak = 0;
        rx.try_recv().ok()
    } else {
        tokio::select! {
            biased;
            _ = run.stop.cancelled() => None,
            offer = gifts.recv() => {
                *gift_streak = gift_streak.saturating_add(1);
                offer
            }
            offer = rx.recv() => {
                *gift_streak = 0;
                offer
            }
        }
    }
}
pub async fn worker(
    app: App,
    run: Arc<Run>,
    mut rx: mpsc::Receiver<Offer>,
    mut gifts: mpsc::Receiver<Offer>,
) {
    let mut gift_streak = 0u8;
    while let Some(o) = next_offer(&run, &mut gifts, &mut rx, &mut gift_streak).await {
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

#[cfg(test)]
mod tests;
