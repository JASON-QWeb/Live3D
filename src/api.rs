use crate::{
    audience, audio,
    config::{self, Settings},
    output, services, session,
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
    Router::new()
        .route("/api/status", get(status))
        .route("/api/obs", post(obs_setup))
        .route("/api/tracks", get(tracks))
        .route(
            "/api/tracks/{value}",
            put(track_add).delete(track_remove).post(track_play),
        )
        .route("/api/choreo", post(choreo))
        .route("/api/soundcheck", post(soundcheck))
        .route("/api/test-audio/{id}", get(test_audio))
        .route("/api/test-audio/{id}/{client}", post(test_receipt))
        .route("/api/balance", get(balance))
        .route("/api/config", get(settings).put(update))
        .route("/api/key/{name}", put(secret))
        .route("/api/avatar", get(avatar).put(upload).delete(remove))
        .route("/api/start", post(start))
        .route("/api/stop", post(stop))
        .route("/api/recover", post(recover))
        .route("/api/command", post(command))
        .route("/api/reference/{kind}", post(reference))
        .route("/api/preview", post(preview))
        .route("/api/media", post(media))
        .route("/api/voice", get(voice).post(launch))
        .route("/api/model", get(model_status).post(model_start))
        .route("/api/preview/reply", post(reply_preview))
        .route("/api/access", get(access))
        .route("/api/frame/{client}/{id}", put(frame))
        .route("/api/monitor", get(monitor))
        .route("/api/output", post(screen))
        .route("/api/ingest", post(incoming))
        .route("/api/simulate", post(simulate))
        .route("/api/recording", post(recording))
        .route("/api/quit", post(quit))
        .route("/sdk/trtc.js", get(sdk))
        .layer(DefaultBodyLimit::max(32 * 1024 * 1024))
}
async fn status(State(app): S) -> Json<Value> {
    Json(json!(app.snapshot()))
}
async fn settings(State(app): S) -> Api<Json<Value>> {
    let mut keys = serde_json::Map::new();
    for n in config::KEY_NAMES {
        keys.insert(n.into(), json!(config::key(n)?.is_some()));
    }
    Ok(Json(
        json!({"settings":app.config(),"keys":keys,"avatar":app.dir.join("avatar.image").is_file(),
        "ffmpeg":audio::ffmpeg().is_some(),"unconfirmed":app.dir.join("open-session.json").is_file(),"data_dir":app.dir}),
    ))
}
async fn update(State(app): S, Json(c): Json<Settings>) -> Api<Json<Value>> {
    let _g = app.gate.lock().await;
    busy(&app)?;
    c.validate()?;
    let lang_changed = app.config().live_language != c.live_language;
    config::write_json(&app.dir.join("settings.json"), &c)?;
    *lock(&app.conf) = c;
    if lang_changed {
        services::ModelService::Voice.stop(&app).await;
    }
    Ok(ok())
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
async fn start(State(app): S) -> Api<Json<Value>> {
    session::start(&app).await?;
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
        "reply" => speech::enqueue(&app, Job::Reply(text(&c.text, 600)?))?,
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
        text: q.text,
        gift,
        kind: Some(q.kind),
        platform: q.platform,
        gift_name: q.gift_name,
        gift_count: q.gift_count,
    };
    Ok(Json(json!({"accepted":audience::receive(&app,event,"simulate")?})))
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
    let cfg = app.config();
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    let generate = crate::dialogue::sentences(
        &app.http,
        &cfg,
        crate::dialogue::PromptKind::Chat,
        &line,
        &[],
        tx,
    );
    let speak = async {
        let mut pcm = Vec::new();
        while let Some(line) = rx.recv().await {
            let part = audio::synth(&app.http, &cfg, &line).await?;
            ensure!(pcm.len() + part.len() < 32_000_000, "回复语音过长");
            pcm.extend_from_slice(&part);
        }
        Ok::<_, anyhow::Error>(pcm)
    };
    let (_, pcm) = tokio::try_join!(generate, speak)?;
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

async fn tracks(State(app): S) -> Api<Json<Value>> {
    Ok(Json(json!(crate::library::list(&app)?)))
}
async fn track_add(State(app): S, Path(title): Path<String>, b: Bytes) -> Api<Json<Value>> {
    Ok(Json(json!(crate::library::add(&app, &title, &b).await?)))
}
async fn track_remove(State(app): S, Path(id): Path<uuid::Uuid>) -> Api<Json<Value>> {
    crate::library::remove(&app, id).await?;
    Ok(ok())
}
async fn track_play(State(app): S, Path(id): Path<uuid::Uuid>) -> Api<Json<Value>> {
    if !crate::library::list(&app)?.iter().any(|t| t.id == id) {
        return Err(anyhow::anyhow!("曲目不存在").into());
    }
    speech::enqueue(&app, Job::Track(id))?;
    Ok(ok())
}
#[derive(Deserialize)]
struct Choreo {
    on: bool,
}
async fn choreo(State(app): S, Json(q): Json<Choreo>) -> Api<Json<Value>> {
    crate::choreo::enable(&app, q.on)?;
    Ok(ok())
}
