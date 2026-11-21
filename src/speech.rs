use crate::{
    audio,
    config::now,
    dialogue,
    session::Run,
    state::{App, lock},
    wire::{Command, Packet},
};
use anyhow::{Context, Result, ensure};
use bytes::Bytes;
use std::{sync::Arc, time::Duration};
use tokio::{sync::mpsc, time::Instant};

pub enum Job {
    Speak(String),
    Reply(String),
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
    if matches!(&job, Job::Speak(_) | Job::Reply(_)) {
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
    let produce = dialogue::sentences(&app.http, &c, text, &history, tx);
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
async fn play(app: &App, run: &Run, o: Offer) -> Result<()> {
    let mut first = Some(o.at);
    match o.job {
        Job::Speak(s) => {
            let pcm = audio::synth(&app.http, &app.config(), &s).await?;
            stream_pcm(app, run, o.epoch, &mut first, pcm, s).await?;
        }
        Job::Reply(s) => reply(app, run, o.epoch, &mut first, &s).await?,
        Job::Track(id) => {
            let pcm = crate::library::audio(app, id).await?;
            stream_pcm(app, run, o.epoch, &mut first, pcm, String::new()).await?;
        }
        Job::Media(pcm, s) => stream_pcm(app, run, o.epoch, &mut first, pcm, s).await?,
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
        lock(&app.view).voice_busy = true;
        tokio::select! {
            biased;
            _ = run.stop.cancelled() => break,
            _ = changes.changed() => (),
            result = play(&app, &run, o) => {
                if let Err(e) = result {
                    app.event("播出错误", e.to_string());
                }
            }
        }
        lock(&app.view).voice_busy = false;
    }
}
