use super::*;

struct AbortOnDrop(Option<tokio::task::JoinHandle<()>>);
impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        if let Some(h) = self.0.take() {
            h.abort();
        }
    }
}

pub(super) fn clear_stage_song(app: &App) {
    let had_song = lock(&app.screen).song.take().is_some();
    if had_song {
        app.notify_stage(json!({"event": "song_stop"}));
    }
}

pub(super) fn notify_bgm_state(app: &App) {
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

pub(super) async fn play_track(
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

    begin_song(app, run, &title);
    let playback_id = uuid::Uuid::new_v4().to_string();
    if use_stage_audio {
        prepare_stage(
            app,
            id,
            &title,
            &playback_id,
            cfg.song_volume as f32 / 100.0,
        );
    }
    sing_intro(app, run, o, at, &cfg, &title, use_stage_audio).await?;
    app.event("唱歌", format!("开始演唱《{title}》"));

    let sync_offset_ms = if use_stage_audio {
        app.config().song_sync_offset_ms as u64
    } else {
        0
    };
    let sing_act = cfg.active_singing_action().trim();
    let sing_cue = (!sing_act.is_empty()).then(|| DelayedCue {
        delay_ms: singing_cue_delay_ms(&pcm, use_stage_audio, sync_offset_ms),
        label: "唱歌神态",
        action: sing_act,
    });
    let sync_guard = use_stage_audio
        .then(|| schedule_stage(app.clone(), playback_id, id, title.clone(), sync_offset_ms));

    let sent_sing_action = stream_pcm_cued(app, run, o, at, pcm, String::new(), sing_cue).await?;

    // Wait the remaining sync tail so the song finishes cleanly before unmuting TRTC or resetting pose.
    let tail_ms = if use_stage_audio {
        sync_offset_ms
    } else {
        SING_CUE_MIN_DELAY_MS
    };
    if tail_ms > 0 {
        tokio::time::sleep(Duration::from_millis(tail_ms)).await;
    }
    drop(sync_guard);
    clear_stage_song(app);
    finish_song(app, run, o, &cfg, &title, sent_sing_action).await
}

fn begin_song(app: &App, run: &Run, title: &str) {
    {
        // Lock song_gifts briefly when entering singing state for strict ordering with gift buffering.
        let _g = lock(&run.song_gifts);
        run.singing.store(true, Ordering::SeqCst);
    }
    {
        let mut v = lock(&app.view);
        v.singing = true;
        v.current_song = title.to_owned();
    }
    // Pause BGM cleanly before the spoken intro and singing start.
    notify_bgm_state(app);
}

fn prepare_stage(app: &App, id: uuid::Uuid, title: &str, playback_id: &str, vol: f32) {
    lock(&app.screen).song = Some(crate::state::StageSong {
        playback_id: playback_id.to_owned(),
        track_id: id.to_string(),
        title: title.to_owned(),
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

async fn sing_intro(
    app: &App,
    run: &Run,
    o: u64,
    at: &mut Option<u64>,
    cfg: &Settings,
    title: &str,
    use_stage_audio: bool,
) -> Result<()> {
    let mut spoke_intro = false;
    if cfg.tts_mode != "system"
        && let Some((_, intro)) = song_phrase(app, run, cfg, Scene::SongIntro, &[("song", title)])
    {
        match audio::synth(&app.http, cfg, &intro).await {
            Ok(intro_pcm) => {
                app.event("开唱报幕", intro.clone());
                stream_pcm(app, run, o, at, intro_pcm, intro).await?;
                spoke_intro = true;
                // Give Vidu a clean beat to finish rendering the spoken intro before starting to sing.
                tokio::time::sleep(Duration::from_millis(INTRO_SETTLE_MS)).await;
            }
            Err(e) => {
                ensure!(o == *run.epoch.borrow(), "播出内容已取消");
                app.event("开唱报幕跳过", format!("语音合成失败，直接开始演唱：{e}"));
            }
        }
    }

    if use_stage_audio && !spoke_intro {
        // Brief window so Stage finishes prebuffering the local Hi-Fi track.
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    Ok(())
}

fn schedule_stage(
    app_clone: App,
    pid: String,
    id: uuid::Uuid,
    song_title: String,
    sync_offset_ms: u64,
) -> AbortOnDrop {
    let tid = id.to_string();
    AbortOnDrop(Some(tokio::spawn(async move {
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
    })))
}

async fn finish_song(
    app: &App,
    run: &Run,
    o: u64,
    cfg: &Settings,
    title: &str,
    sent_sing_action: bool,
) -> Result<()> {
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
    } else if sent_sing_action {
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
        let gifts = crate::director::format_gift_summary(cfg, &buffered_gifts);
        if let Some((profile, line)) =
            song_phrase(app, run, cfg, scene, &[("song", title), ("gifts", &gifts)])
        {
            let mut thanks_at = Some(now());
            let mut playback = ReplyPlayback {
                app,
                run,
                cfg,
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
