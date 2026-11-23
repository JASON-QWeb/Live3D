use crate::{
    config::{self, Settings, Tokens, now},
    prompt_store,
    session::Run,
};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard},
};
use tokio_util::sync::CancellationToken;

pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
#[derive(Clone, Serialize)]
pub struct Event {
    pub at: u64,
    pub kind: String,
    pub text: String,
}
#[derive(Clone, Default, Serialize)]
pub struct Metrics {
    pub pcm_bytes: u64,
    pub utterances: u32,
    pub actions_sent: u32,
    pub interrupts: u32,
    pub reconnects: u32,
    pub inputs: u32,
    pub last_first_audio_ms: Option<u64>,
    pub last_reply: Option<ReplyMetrics>,
    pub first_frame_ms: Option<u64>,
    pub billed_seconds: Option<u64>,
    pub credits: Option<f64>,
    pub output_fps: Option<f64>,
    pub output_width: Option<u64>,
    pub output_height: Option<u64>,
}

#[derive(Clone, Default, Serialize)]
pub struct ReplyMetrics {
    pub source: &'static str,
    pub queue_ms: Option<u64>,
    pub tts_ms: Option<u64>,
    pub first_audio_ms: Option<u64>,
    #[serde(flatten)]
    pub generation: crate::dialogue::GenerationStats,
}
#[derive(Clone, Default, Serialize)]
pub struct View {
    pub phase: String,
    pub message: String,
    pub live_id: String,
    pub started: u64,
    pub ended: u64,
    pub subtitle: String,
    pub subtitle_until: u64,
    pub metrics: Metrics,
    pub events: VecDeque<Event>,
    pub output: String,
    pub input: String,
    /// Douyin comment collector state; empty when no room is set.
    pub douyin: String,
    pub pending: u32,
    pub voice_busy: bool,
    pub singing: bool,
    pub current_song: String,
    pub bgm_playing: bool,
    pub current_bgm: String,
    pub bgm_status: String,
    pub choreo: bool,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Rtc {
    pub app_id: String,
    pub channel_id: String,
    pub user_id: String,
    pub token: String,
}
#[derive(Clone, Serialize)]
pub struct StageSong {
    pub playback_id: String,
    pub track_id: String,
    pub title: String,
    pub state: String, // "prepare" | "play"
    pub started_at: u64,
    pub volume: f32,
}
#[derive(Default)]
pub struct Screen {
    pub lease: String,
    pub seen: u64,
    pub rtc: Option<Rtc>,
    pub tone: Option<crate::soundcheck::Tone>,
    pub poster: Option<(u64, bytes::Bytes)>,
    pub song: Option<StageSong>,
    pub dance: Option<crate::dance::Playback>,
    pub bgm_enabled: bool,
    pub bgm_index: usize,
    pub requested_bgm: Option<crate::library::BgmTrack>,
    pub bgm_requests: VecDeque<crate::library::BgmTrack>,
    pub failed_bgm: std::collections::HashSet<uuid::Uuid>,
    pub bgm_revision: u64,
}
pub struct Inner {
    pub dir: PathBuf,
    pub tokens: Tokens,
    pub conf: Mutex<Settings>,
    pub prompts: prompt_store::Store,
    pub view: Mutex<View>,
    pub screen: Mutex<Screen>,
    pub run: Mutex<Option<Arc<Run>>>,
    pub gate: tokio::sync::Mutex<()>,
    pub http: reqwest::Client,
    pub quit: CancellationToken,
    pub voice_process: tokio::sync::Mutex<Option<tokio::process::Child>>,
    pub chat_process: tokio::sync::Mutex<Option<tokio::process::Child>>,
    pub obs_gate: tokio::sync::Mutex<()>,
    pub music_lookups: Arc<tokio::sync::Semaphore>,
    pub library_gate: tokio::sync::Mutex<()>,
    pub sdk_gate: tokio::sync::Mutex<()>,
    pub seen: Mutex<VecDeque<String>>,
    pub stage_tx: tokio::sync::broadcast::Sender<Value>,
    pub boot_id: String,
}
pub type App = Arc<Inner>;
impl Inner {
    pub fn new(dir: PathBuf) -> Result<App> {
        config::prepare_dir(&dir)?;
        let (conf, legacy) = config::load_settings(&dir)?;
        let prompts = prompt_store::Store::new(&dir, &conf.prompt_profile, &legacy)?;
        let pending = dir.join("open-session.json").exists();
        let view = View {
            phase: if pending { "unconfirmed" } else { "idle" }.into(),
            message: if pending {
                "上次会话结束状态待核对"
            } else {
                "等待连线"
            }
            .into(),
            ..View::default()
        };
        let (stage_tx, _) = tokio::sync::broadcast::channel(32);
        Ok(Arc::new(Self {
            conf: Mutex::new(conf),
            prompts,
            tokens: Tokens::load(&dir)?,
            dir,
            view: Mutex::new(view),
            screen: Mutex::new(Screen::default()),
            run: Mutex::new(None),
            gate: tokio::sync::Mutex::new(()),
            http: reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(8))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            quit: CancellationToken::new(),
            voice_process: tokio::sync::Mutex::new(None),
            chat_process: tokio::sync::Mutex::new(None),
            obs_gate: tokio::sync::Mutex::new(()),
            music_lookups: Arc::new(tokio::sync::Semaphore::new(4)),
            library_gate: tokio::sync::Mutex::new(()),
            sdk_gate: tokio::sync::Mutex::new(()),
            seen: Mutex::new(VecDeque::new()),
            stage_tx,
            boot_id: uuid::Uuid::new_v4().to_string(),
        }))
    }
    pub fn notify_stage(&self, payload: Value) {
        let _ = self.stage_tx.send(payload);
    }
    pub fn config(&self) -> Settings {
        lock(&self.conf).clone()
    }
    pub fn phase(&self, phase: &str, message: &str) {
        let mut v = lock(&self.view);
        v.phase = phase.into();
        v.message = message.into();
    }
    pub fn event(&self, kind: impl Into<String>, msg: impl Into<String>) {
        let mut v = lock(&self.view);
        if v.events.len() >= 120 {
            v.events.pop_front();
        }
        v.events.push_back(Event {
            at: now(),
            kind: kind.into(),
            text: msg.into(),
        });
    }
    pub fn active(&self) -> anyhow::Result<Arc<Run>> {
        anyhow::ensure!(lock(&self.view).phase == "live", "请先建立数字人连线");
        lock(&self.run)
            .clone()
            .ok_or_else(|| anyhow::anyhow!("会话未就绪"))
    }
    pub fn snapshot(&self) -> View {
        let mut v = lock(&self.view).clone();
        if now().saturating_sub(lock(&self.screen).seen) > 12_000 {
            v.output.clear();
            v.bgm_status.clear();
        }
        if v.subtitle_until < now() {
            v.subtitle.clear();
        }
        v
    }
    pub fn journal(&self, v: &Value) -> Result<()> {
        config::write_json(&self.dir.join("open-session.json"), v)
    }
}
