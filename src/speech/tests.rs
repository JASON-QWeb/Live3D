use super::*;
use crate::state::Inner;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicU64, AtomicUsize},
};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

fn pcm_frames(frames: &[i16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(frames.len() * audio::FRAME);
    for &amp in frames {
        let bytes = amp.to_le_bytes();
        for _ in 0..(audio::FRAME / 2) {
            out.extend_from_slice(&bytes);
        }
    }
    out
}

fn test_run() -> (App, Arc<Run>, mpsc::Receiver<Packet>) {
    let dir = tempfile::tempdir().unwrap();
    let app = Inner::new(dir.path().to_path_buf()).unwrap();
    let (wire, wrx) = mpsc::channel(32);
    let (jobs, _jrx) = mpsc::channel(4);
    let (gifts, _grx) = mpsc::channel(4);
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
        singing: AtomicBool::new(false),
        busy: AtomicBool::new(false),
        last_activity_ms: AtomicU64::new(0),
        last_song_end_ms: AtomicU64::new(0),
        last_ambient_ms: AtomicU64::new(0),
        last_join_welcome_ms: AtomicU64::new(0),
        song_gifts: Mutex::new(Vec::new()),
        next_track_index: AtomicUsize::new(0),
    });
    (app, run, wrx)
}

#[test]
fn leading_silence_skips_quiet_intro_and_transient_clicks() {
    assert_eq!(leading_silence_ms(&[]), 0);

    let immediate = pcm_frames(&[1200, 1200, 1200, 1200]);
    assert_eq!(leading_silence_ms(&immediate), 0);

    // 5 silent frames, 1 transient click, 4 silent frames, then continuous audio at frame 10 (200 ms)
    let mut amps = vec![0; 5];
    amps.push(2000);
    amps.extend(vec![10; 4]);
    amps.extend(vec![1000; 5]);
    assert_eq!(leading_silence_ms(&pcm_frames(&amps)), 200);

    // All silent falls back to 0 so the cue still fires at base delay
    assert_eq!(leading_silence_ms(&pcm_frames(&[0; 20])), 0);
}

#[test]
fn singing_cue_delay_waits_for_stage_offset_and_leading_silence() {
    let mut amps = vec![0; 15]; // 300 ms silence
    amps.extend(vec![1500; 5]);
    let pcm = pcm_frames(&amps);

    assert_eq!(singing_cue_delay_ms(&pcm, true, 1200), 1500);
    assert_eq!(singing_cue_delay_ms(&pcm, true, 400), 1100);
    assert_eq!(singing_cue_delay_ms(&pcm, false, 1200), 1100);
}

#[tokio::test]
async fn stream_pcm_cued_emits_signal_9_only_after_delay_frames() {
    let (app, run, mut wrx) = test_run();
    let pcm = Bytes::from(pcm_frames(&[1000; 5])); // 100 ms total
    let mut at = Some(now());

    let sent = stream_pcm_cued(
        &app,
        &run,
        0,
        &mut at,
        pcm,
        String::new(),
        Some(DelayedCue {
            delay_ms: 40,
            label: "唱歌神态",
            action: "自然投入地唱歌",
        }),
    )
    .await
    .unwrap();
    assert!(sent);

    let mut audio_before_signal = 0usize;
    let mut audio_after_signal = 0usize;
    let mut signal_texts = Vec::new();
    while let Ok(pkt) = wrx.try_recv() {
        match pkt.cmd {
            Command::Line(_, _) => {}
            Command::Audio(_) => {
                if signal_texts.is_empty() {
                    audio_before_signal += 1;
                } else {
                    audio_after_signal += 1;
                }
            }
            Command::Signal(9, payload) => {
                signal_texts.push(payload["text_msg"]["content"].as_str().unwrap().to_owned());
            }
            Command::Signal(other, _) => panic!("unexpected signal type {other}"),
        }
    }

    // Frame indices 0 (0ms), 1 (20ms), 2 (40ms -> cue fires after frame 2 audio)
    assert_eq!(audio_before_signal, 3);
    assert_eq!(audio_after_signal, 2);
    assert_eq!(signal_texts, vec!["自然投入地唱歌"]);
}

#[tokio::test]
async fn stream_pcm_cued_skips_signal_if_track_ends_before_delay() {
    let (app, run, mut wrx) = test_run();
    let pcm = Bytes::from(pcm_frames(&[1000; 2])); // 40 ms total (frames at 0ms, 20ms)
    let mut at = Some(now());

    let sent = stream_pcm_cued(
        &app,
        &run,
        0,
        &mut at,
        pcm,
        String::new(),
        Some(DelayedCue {
            delay_ms: 100,
            label: "唱歌神态",
            action: "自然投入地唱歌",
        }),
    )
    .await
    .unwrap();
    assert!(!sent);

    while let Ok(pkt) = wrx.try_recv() {
        assert!(!matches!(pkt.cmd, Command::Signal(9, _)));
    }
}
