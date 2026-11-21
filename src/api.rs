use crate::{
    audience, audio,
    config::{self, Settings},
    output, prompt_store, services, session,
    speech::{self, Job},
    state::{App, lock},
    vidu, wire,
};
use anyhow::{Context, Result, ensure};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use serde::Deserialize;
use serde_json::{Value, json};

pub struct Problem(anyhow::Error);
impl From<anyhow::Error> for Problem {
    fn from(e: anyhow::Error) -> Self {
        Self(e)
    }
}
impl IntoResponse for Problem {
    fn into_response(self) -> Response {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":self.0.to_string()})),
        )
            .into_response()
    }
}
type Api<T> = std::result::Result<T, Problem>;
type S = State<App>;
fn ok() -> Json<Value> {
    Json(json!({"ok":true}))
}
fn asset(mime: &'static str, b: impl Into<axum::body::Body>) -> Response {
    let mut r = Response::new(b.into());
    r.headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
    r
}
fn busy(app: &App) -> Result<()> {
    ensure!(lock(&app.run).is_none(), "请先结束当前连线再修改设置");
    Ok(())
}
pub fn routes() -> Router<App> {
    let audio_uploads = Router::new()
        .route(
            "/api/tracks/{value}",
            put(track_add).delete(track_remove).post(track_play),
        )
        .route("/api/bgm/{value}", put(bgm_add).delete(bgm_remove))
        .route("/api/media", post(media))
        .layer(DefaultBodyLimit::max(audio::MAX_AUDIO));

    Router::new()
        .route("/api/status", get(status))
        .route("/api/obs", post(obs_setup))
        .route("/api/tracks", get(tracks).delete(tracks_clear))
        .route("/api/tracks/{id}/audio", get(track_audio))
        .route("/api/tracks/{id}/wav", get(track_wav))
        .route("/api/bgm", get(bgm_tracks).delete(bgm_clear))
        .route("/api/bgm/{id}/audio", get(bgm_audio))
        .route("/api/bgm/{id}/wav", get(bgm_wav))
        .route("/api/bgm/control", post(bgm_control))
        .route("/api/audio-live", post(audio_live))
        .route("/api/stage/events", get(stage_events))
        .route("/api/stage/track/{id}", get(track_audio))
        .route("/api/stage/track/{id}/wav", get(track_wav))
        .route("/api/stage/bgm/{id}", get(bgm_audio))
        .route("/api/stage/bgm/{id}/wav", get(bgm_wav))
        .route("/api/stage/bgm/next", post(stage_bgm_next))
        .route("/api/choreo", post(choreo))
        .route("/api/soundcheck", post(soundcheck))
        .route("/api/test-audio/{id}", get(test_audio))
        .route("/api/test-audio/{id}/{client}", post(test_receipt))
        .route("/api/balance", get(balance))
        .route("/api/config", get(settings).put(update))
        .route(
            "/api/prompts/{id}/{language}",
            get(prompt_document).put(save_prompt),
        )
        .route("/api/key/{name}", put(secret))
        .route("/api/avatar", get(avatar).put(upload).delete(remove))
        .route("/api/start", post(start))
        .route("/api/stop", post(stop))
        .route("/api/recover", post(recover))
        .route("/api/command", post(command))
        .route("/api/reference/{kind}", post(reference))
        .route("/api/preview", post(preview))
        .route("/api/voice", get(voice).post(launch))
        .route("/api/model", get(model_status).post(model_start))
        .route("/api/preview/reply", post(reply_preview))
        .route("/api/access", get(access))
        .route("/api/frame/{client}/{id}", put(frame))
        .route("/api/monitor", get(monitor))
        .route("/api/output", post(screen))
        .route("/api/ingest", post(incoming))
        .route("/api/simulate", post(simulate))
        .route("/api/events/clear", post(events_clear))
        .route("/api/recording", post(recording))
        .route("/api/quit", post(quit))
        .route("/sdk/trtc.js", get(sdk))
        .layer(DefaultBodyLimit::max(32 * 1024 * 1024))
        .merge(audio_uploads)
}
async fn status(State(app): S) -> Json<Value> {
    Json(json!(app.snapshot()))
}
async fn settings(State(app): S) -> Api<Json<Value>> {
    let mut keys = serde_json::Map::new();
    for n in config::KEY_NAMES {
        keys.insert(n.into(), json!(config::key(n)?.is_some()));
    }
    let cfg = app.config();
    Ok(Json(
        json!({"settings":cfg,"keys":keys,"avatar":app.dir.join("avatar.image").is_file(),
        "ffmpeg":audio::ffmpeg().is_some(),"unconfirmed":app.dir.join("open-session.json").is_file(),"data_dir":app.dir}),
    ))
}
async fn update(State(app): S, Json(c): Json<Settings>) -> Api<Json<Value>> {
    let _g = app.gate.lock().await;
    busy(&app)?;
    c.validate()?;
    let previous = app.config();
    let lang_changed = previous.live_language != c.live_language;
    if previous.prompt_profile != c.prompt_profile {
        app.prompts.prepare(&c.prompt_profile, None)?;
    }
    config::write_json(&app.dir.join("settings.json"), &c)?;
    *lock(&app.conf) = c;
    if lang_changed {
        services::ModelService::Voice.stop(&app).await;
    }
    Ok(ok())
}

fn profile_language(app: &App, id: &str, language: &str) -> Result<bool> {
    ensure!(
        lock(&app.conf).prompt_profile == id,
        "当前话术档案已切换，请重新加载设置"
    );
    match language {
        "zh" => Ok(false),
        "en" => Ok(true),
        _ => anyhow::bail!("话术档案语言无效"),
    }
}

async fn prompt_document(
    State(app): S,
    Path((id, language)): Path<(String, String)>,
) -> Api<Json<prompt_store::Document>> {
    let english = profile_language(&app, &id, &language)?;
    Ok(Json(app.prompts.document(&id, english)?))
}

async fn save_prompt(
    State(app): S,
    Path((id, language)): Path<(String, String)>,
    Json(edit): Json<prompt_store::Edit>,
) -> Api<Json<prompt_store::Document>> {
    let _guard = app.gate.lock().await;
    let english = profile_language(&app, &id, &language)?;
    Ok(Json(app.prompts.save(&id, english, edit)?))
}
#[derive(Deserialize)]
struct Secret {
    value: String,
}
async fn secret(State(app): S, Path(n): Path<String>, Json(v): Json<Secret>) -> Api<Json<Value>> {
    let _g = app.gate.lock().await;
    busy(&app)?;
    config::store_key(&n, &v.value)?;
    Ok(ok())
}
async fn upload(State(app): S, b: Bytes) -> Api<Json<Value>> {
    let _g = app.gate.lock().await;
    busy(&app)?;
    config::image_type(&b)?;
    config::atomic(&app.dir.join("avatar.image"), &b)?;
    Ok(ok())
}
async fn remove(State(app): S) -> Api<Json<Value>> {
    let _g = app.gate.lock().await;
    busy(&app)?;
    let p = app.dir.join("avatar.image");
    if p.exists() {
        std::fs::remove_file(p).context("无法移除角色图")?;
    }
    Ok(ok())
}
async fn avatar(State(app): S) -> Api<Response> {
    let p = app.dir.join("avatar.image");
    if !p.exists() {
        return Ok(StatusCode::NOT_FOUND.into_response());
    }
    let b = std::fs::read(p).context("无法读取角色图")?;
    Ok(asset(config::image_type(&b)?, b))
}
#[derive(Deserialize, Default)]
struct StartReq {
    #[serde(default)]
    keep_events: bool,
}
async fn start(State(app): S, q: Option<Json<StartReq>>) -> Api<Json<Value>> {
    let keep_events = q.map(|Json(v)| v.keep_events).unwrap_or(false);
    session::start(&app, keep_events).await?;
    Ok(ok())
}
async fn events_clear(State(app): S) -> Api<Json<Value>> {
    lock(&app.view).events.clear();
    lock(&app.seen).clear();
    Ok(ok())
}
async fn stop(State(app): S) -> Api<Json<Value>> {
    session::stop(&app)?;
    Ok(ok())
}
#[derive(Deserialize)]
struct Recovery {
    #[serde(default)]
    operator_confirmed: bool,
}
async fn recover(State(app): S, Json(q): Json<Recovery>) -> Api<Json<Value>> {
    session::recover(&app, q.operator_confirmed).await?;
    Ok(ok())
}
#[derive(Deserialize)]
struct Act {
    kind: String,
    #[serde(default)]
    text: String,
}
fn text(s: &str, max: usize) -> Result<String> {
    let v = s.trim();
    ensure!(
        !v.is_empty() && v.chars().count() <= max,
        "请输入 1 至 {max} 字"
    );
    Ok(v.to_owned())
}
async fn command(State(app): S, Json(c): Json<Act>) -> Api<Json<Value>> {
    match c.kind.as_str() {
        "say" => speech::enqueue(&app, Job::Speak(text(&c.text, 600)?))?,
        "reply" => speech::enqueue(&app, Job::Reply(text(&c.text, 600)?.into()))?,
        "action" => {
            let s = text(&c.text, 500)?;
            let run = app.active()?;
            crate::choreo::hold(&app, &run);
            session::signal(&app, 9, wire::text_msg(&s))?;
            app.event("动作", s);
        }
        "interrupt" => {
            let run = app.active()?;
            speech::interrupt(&app, &run);
            app.event("播出", "操作员打断播出");
        }
        "reset" => session::signal(&app, 15, json!({}))?,
        "remove_reference" => {
            session::signal(&app, 11, json!({"prompt_operation":{"op_type":"remove"}}))?
        }
        _ => return Err(anyhow::anyhow!("未知操作").into()),
    }
    Ok(ok())
}
async fn reference(State(app): S, Path(kind): Path<String>, b: Bytes) -> Api<Json<Value>> {
    if !["object", "garment", "background"].contains(&kind.as_str()) {
        return Err(anyhow::anyhow!("参考图类型无效").into());
    }
    let im = json!({"image_uri":config::image_uri(&b)?,"image_id":uuid::Uuid::new_v4().to_string(),"kind":kind});
    session::signal(
        &app,
        11,
        json!({"prompt_operation":{"op_type":"switch","images":[im]}}),
    )?;
    Ok(ok())
}
#[derive(Deserialize)]
struct Words {
    text: String,
}
async fn preview(State(app): S, Json(q): Json<Words>) -> Api<Response> {
    let line = text(&q.text, 500)?;
    let pcm = audio::synth(&app.http, &app.config(), &line).await?;
    Ok(asset("audio/wav", audio::wav(&pcm)))
}
async fn media(State(app): S, b: Bytes) -> Api<Json<Value>> {
    let run = app.active()?;
    let epoch = *run.epoch.borrow();
    let pcm = audio::decode(&b).await?;
    let current = app.active()?;
    if !std::sync::Arc::ptr_eq(&current, &run) || epoch != *current.epoch.borrow() {
        return Err(anyhow::anyhow!("会话或播出队列已经切换，请重新选择音频").into());
    }
    speech::enqueue_in(&app, &run, Job::Media(pcm, String::new()), false)?;
    Ok(ok())
}
async fn voice(State(app): S) -> Json<Value> {
    Json(json!({"message":services::ModelService::Voice.status(&app).await}))
}
async fn launch(State(app): S) -> Api<Json<Value>> {
    Ok(Json(
        json!({"message":services::ModelService::Voice.launch(&app).await?}),
    ))
}
async fn access(State(app): S) -> Json<Value> {
    Json(json!({"stage":app.tokens.stage,"ingest":app.tokens.ingest}))
}
async fn screen(State(app): S, Json(q): Json<output::Report>) -> Api<Json<Value>> {
    Ok(Json(output::heartbeat(&app, q)?))
}
async fn incoming(State(app): S, Json(q): Json<audience::Incoming>) -> Api<Json<Value>> {
    Ok(Json(json!({"accepted":audience::receive(&app,q,"local")?})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SimulatedEvent {
    #[serde(default)]
    platform: Option<String>,
    kind: String,
    author: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    gift_name: Option<String>,
    #[serde(default)]
    gift_count: Option<u32>,
}
async fn simulate(State(app): S, Json(q): Json<SimulatedEvent>) -> Api<Json<Value>> {
    let gift = q.kind.trim() == "gift";
    let event = audience::Incoming {
        id: format!("sim-{}", uuid::Uuid::new_v4()),
        author: q.author,
        viewer_id: None,
        text: q.text,
        gift,
        kind: Some(q.kind),
        platform: q.platform,
        gift_name: q.gift_name,
        gift_count: q.gift_count,
    };
    Ok(Json(
        json!({"accepted":audience::receive(&app,event,"simulate")?}),
    ))
}
#[derive(Deserialize)]
struct Recording {
    id: String,
}
async fn recording(State(app): S, Json(q): Json<Recording>) -> Api<Json<Value>> {
    let k = config::key("vidu")?.context("请填写 Vidu 密钥")?;
    Ok(Json(vidu::query(&app, &k, &q.id, true).await?))
}
async fn quit(State(app): S) -> Api<Json<Value>> {
    app.quit.cancel();
    Ok(ok())
}
async fn sdk(State(app): S) -> Api<Response> {
    Ok(asset(
        "text/javascript; charset=utf-8",
        services::sdk(&app).await?,
    ))
}

async fn frame(
    State(app): S,
    Path((client, id)): Path<(String, String)>,
    b: Bytes,
) -> Api<Json<Value>> {
    if b.len() > 180_000 || !b.starts_with(&[255, 216, 255]) {
        return Err(anyhow::anyhow!("监看帧无效").into());
    }
    let current = lock(&app.view).live_id.clone();
    let mut sc = lock(&app.screen);
    if sc.lease != client || sc.rtc.is_none() || current != id {
        return Err(anyhow::anyhow!("输出端已切换").into());
    }
    sc.poster = Some((config::now(), b));
    Ok(ok())
}
async fn monitor(State(app): S) -> Response {
    let sc = lock(&app.screen);
    if sc.rtc.is_some()
        && let Some((at, b)) = &sc.poster
        && config::now().saturating_sub(*at) < 6500
    {
        return asset("image/jpeg", b.clone());
    }
    StatusCode::NO_CONTENT.into_response()
}

async fn balance(State(app): S) -> Api<Json<Value>> {
    Ok(Json(vidu::balance(&app).await?))
}

async fn model_status(State(app): S) -> Json<Value> {
    Json(json!({"message":services::ModelService::Chat.status(&app).await}))
}
async fn model_start(State(app): S) -> Api<Json<Value>> {
    Ok(Json(
        json!({"message":services::ModelService::Chat.launch(&app).await?}),
    ))
}
async fn reply_preview(State(app): S, Json(q): Json<Words>) -> Api<Response> {
    let line = text(&q.text, 500)?;
    let pcm = speech::preview_reply(&app, line).await?;
    Ok(asset("audio/wav", audio::wav(&pcm)))
}

#[derive(Deserialize)]
struct Obs {
    port: u16,
    password: String,
}
async fn obs_setup(
    State(app): S,
    h: axum::http::HeaderMap,
    Json(q): Json<Obs>,
) -> Api<Json<Value>> {
    let host = h
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .context("缺少本机地址")?;
    Ok(Json(
        crate::obs::setup(&app, q.port, &q.password, host).await?,
    ))
}
async fn soundcheck(State(app): S, Json(q): Json<Words>) -> Api<Json<Value>> {
    let line = text(&q.text, 500)?;
    crate::soundcheck::send(&app, &line).await?;
    Ok(ok())
}
async fn test_audio(State(app): S, Path(id): Path<String>) -> Response {
    match crate::soundcheck::read(&app, &id) {
        Some(b) => asset("audio/wav", b),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
async fn test_receipt(
    State(app): S,
    Path((id, client)): Path<(String, String)>,
) -> Api<Json<Value>> {
    crate::soundcheck::receipt(&app, &id, &client)?;
    Ok(ok())
}

fn ext_header(h: &axum::http::HeaderMap) -> Option<&str> {
    h.get("x-file-ext")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.len() <= 16)
}

async fn tracks(State(app): S) -> Api<Json<Value>> {
    Ok(Json(json!(crate::library::list(&app)?)))
}
async fn track_add(
    State(app): S,
    h: axum::http::HeaderMap,
    Path(title): Path<String>,
    b: Bytes,
) -> Api<Json<Value>> {
    Ok(Json(json!(
        crate::library::add(&app, &title, ext_header(&h), &b).await?
    )))
}
async fn track_remove(State(app): S, Path(id): Path<uuid::Uuid>) -> Api<Json<Value>> {
    crate::library::remove(&app, id).await?;
    Ok(ok())
}
async fn tracks_clear(State(app): S) -> Api<Json<Value>> {
    let removed = crate::library::clear(&app).await?;
    Ok(Json(json!({"ok": true, "removed": removed})))
}
async fn track_play(State(app): S, Path(id): Path<uuid::Uuid>) -> Api<Json<Value>> {
    if !crate::library::list(&app)?.iter().any(|t| t.id == id) {
        return Err(anyhow::anyhow!("曲目不存在").into());
    }
    speech::enqueue(&app, Job::Track(id))?;
    Ok(ok())
}
async fn track_audio(State(app): S, Path(id): Path<uuid::Uuid>) -> Api<Response> {
    let (mime, bytes) = crate::library::playback_audio(&app, id).await?;
    Ok(asset(mime, bytes))
}
async fn track_wav(State(app): S, Path(id): Path<uuid::Uuid>) -> Api<Response> {
    let bytes = crate::library::playback_wav(&app, id).await?;
    Ok(asset("audio/wav", bytes))
}

async fn bgm_tracks(State(app): S) -> Api<Json<Value>> {
    Ok(Json(json!(crate::library::bgm_list(&app)?)))
}
async fn bgm_add(
    State(app): S,
    h: axum::http::HeaderMap,
    Path(title): Path<String>,
    b: Bytes,
) -> Api<Json<Value>> {
    let item = crate::library::bgm_add(&app, &title, ext_header(&h), &b).await?;
    app.notify_stage(json!({"event": "bgm_state", "bgm": output::bgm_snapshot(&app)}));
    Ok(Json(json!(item)))
}
async fn bgm_remove(State(app): S, Path(id): Path<uuid::Uuid>) -> Api<Json<Value>> {
    crate::library::bgm_remove(&app, id).await?;
    {
        let mut s = lock(&app.screen);
        s.bgm_revision = s.bgm_revision.wrapping_add(1);
    }
    app.notify_stage(json!({"event": "bgm_state", "bgm": output::bgm_snapshot(&app)}));
    Ok(ok())
}
async fn bgm_clear(State(app): S) -> Api<Json<Value>> {
    let removed = crate::library::bgm_clear(&app).await?;
    {
        let mut s = lock(&app.screen);
        s.bgm_enabled = false;
        s.bgm_index = 0;
        s.bgm_revision = s.bgm_revision.wrapping_add(1);
    }
    app.notify_stage(json!({"event": "bgm_state", "bgm": output::bgm_snapshot(&app)}));
    Ok(Json(json!({"ok": true, "removed": removed})))
}
async fn bgm_audio(State(app): S, Path(id): Path<uuid::Uuid>) -> Api<Response> {
    let (mime, bytes) = crate::library::bgm_audio(&app, id).await?;
    Ok(asset(mime, bytes))
}
async fn bgm_wav(State(app): S, Path(id): Path<uuid::Uuid>) -> Api<Response> {
    let bytes = crate::library::bgm_wav(&app, id).await?;
    Ok(asset("audio/wav", bytes))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BgmControl {
    action: String,
    #[serde(default)]
    id: Option<uuid::Uuid>,
}
async fn bgm_control(State(app): S, Json(q): Json<BgmControl>) -> Api<Json<Value>> {
    let items = crate::library::bgm_list(&app)?;
    match q.action.as_str() {
        "toggle" => {
            let mut s = lock(&app.screen);
            if !s.bgm_enabled {
                if items.is_empty() {
                    return Err(anyhow::anyhow!("请先导入背景音乐").into());
                }
                s.bgm_enabled = true;
            } else {
                s.bgm_enabled = false;
            }
            s.bgm_revision = s.bgm_revision.wrapping_add(1);
        }
        "start" => {
            if items.is_empty() {
                return Err(anyhow::anyhow!("请先导入背景音乐").into());
            }
            let mut s = lock(&app.screen);
            s.bgm_enabled = true;
            s.bgm_revision = s.bgm_revision.wrapping_add(1);
        }
        "stop" => {
            let mut s = lock(&app.screen);
            s.bgm_enabled = false;
            s.bgm_revision = s.bgm_revision.wrapping_add(1);
        }
        "next" => {
            if items.is_empty() {
                return Err(anyhow::anyhow!("背景音乐列表为空").into());
            }
            let mut s = lock(&app.screen);
            s.bgm_index = (s.bgm_index + 1) % items.len();
            s.bgm_revision = s.bgm_revision.wrapping_add(1);
        }
        "select" => {
            let id = q.id.context("请选择要播放的背景音乐")?;
            let pos = items
                .iter()
                .position(|t| t.id == id)
                .context("找不到该背景音乐")?;
            let mut s = lock(&app.screen);
            s.bgm_index = pos;
            s.bgm_enabled = true;
            s.bgm_revision = s.bgm_revision.wrapping_add(1);
        }
        _ => return Err(anyhow::anyhow!("未知背景音乐操作").into()),
    }
    let bgm = output::bgm_snapshot(&app);
    app.notify_stage(json!({"event": "bgm_state", "bgm": bgm}));
    Ok(Json(json!({"ok": true, "bgm": bgm})))
}

#[derive(Deserialize)]
struct StageClient {
    client: String,
}
async fn stage_bgm_next(State(app): S, Json(q): Json<StageClient>) -> Api<Json<Value>> {
    let items = crate::library::bgm_list(&app)?;
    let is_owner = {
        let mut s = lock(&app.screen);
        if s.lease != q.client {
            false
        } else {
            if !items.is_empty() && s.bgm_enabled {
                s.bgm_index = (s.bgm_index + 1) % items.len();
                s.bgm_revision = s.bgm_revision.wrapping_add(1);
            }
            true
        }
    };
    if !is_owner {
        return Ok(Json(json!({"ok": false})));
    }
    let bgm = output::bgm_snapshot(&app);
    app.notify_stage(json!({"event": "bgm_state", "bgm": bgm}));
    Ok(Json(json!({"ok": true, "bgm": bgm})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AudioLiveUpdate {
    #[serde(default)]
    song_audio_mode: Option<String>,
    #[serde(default)]
    song_sync_offset_ms: Option<u32>,
    #[serde(default)]
    song_volume: Option<u32>,
    #[serde(default)]
    bgm_volume: Option<u32>,
    #[serde(default)]
    bgm_duck_percent: Option<u32>,
}
async fn audio_live(State(app): S, Json(q): Json<AudioLiveUpdate>) -> Api<Json<Value>> {
    let mut cfg = app.config();
    if let Some(m) = q.song_audio_mode {
        cfg.song_audio_mode = m;
    }
    if let Some(ms) = q.song_sync_offset_ms {
        cfg.song_sync_offset_ms = ms;
    }
    if let Some(v) = q.song_volume {
        cfg.song_volume = v;
    }
    if let Some(v) = q.bgm_volume {
        cfg.bgm_volume = v;
    }
    if let Some(v) = q.bgm_duck_percent {
        cfg.bgm_duck_percent = v;
    }
    cfg.validate()?;
    let song_vol = cfg.song_volume as f32 / 100.0;
    config::write_json(&app.dir.join("settings.json"), &cfg)?;
    *lock(&app.conf) = cfg;
    {
        if let Some(ref mut s) = lock(&app.screen).song {
            s.volume = song_vol;
        }
    }
    app.notify_stage(json!({"event": "song_volume", "volume": song_vol}));
    app.notify_stage(json!({"event": "bgm_state", "bgm": output::bgm_snapshot(&app)}));
    Ok(ok())
}

async fn stage_events(State(app): S) -> Response {
    let rx = app.stage_tx.subscribe();
    let initial = json!({"event": "bgm_state", "bgm": output::bgm_snapshot(&app)});
    let stream =
        futures_util::stream::unfold((Some(initial), rx), |(mut first, mut rx)| async move {
            if let Some(init) = first.take() {
                let chunk = Bytes::from(format!("data: {init}\n\n"));
                return Some((Ok::<_, std::convert::Infallible>(chunk), (None, rx)));
            }
            loop {
                match tokio::time::timeout(std::time::Duration::from_secs(15), rx.recv()).await {
                    Ok(Ok(val)) => {
                        let chunk = Bytes::from(format!("data: {val}\n\n"));
                        return Some((Ok(chunk), (None, rx)));
                    }
                    Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
                    Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => return None,
                    Err(_) => {
                        return Some((Ok(Bytes::from_static(b": ping\n\n")), (None, rx)));
                    }
                }
            }
        });
    let mut r = Response::new(axum::body::Body::from_stream(stream));
    r.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream; charset=utf-8"),
    );
    r.headers_mut().insert(
        header::HeaderName::from_static("x-accel-buffering"),
        HeaderValue::from_static("no"),
    );
    r
}

#[derive(Deserialize)]
struct Choreo {
    on: bool,
}
async fn choreo(State(app): S, Json(q): Json<Choreo>) -> Api<Json<Value>> {
    crate::choreo::enable(&app, q.on)?;
    Ok(ok())
}
