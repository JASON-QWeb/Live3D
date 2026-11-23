use crate::{
    api::Problem,
    audio, config,
    state::{App, lock},
};
use anyhow::{Context, Result, ensure};
use axum::{
    Json, Router,
    body::Body,
    extract::{Path, State},
    http::{Request, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{path::PathBuf, time::Duration};
use tokio::io::AsyncWriteExt;
use tower_http::services::ServeFile;
use uuid::Uuid;

#[derive(Clone, Deserialize, Serialize)]
pub struct Clip {
    id: Uuid,
    title: String,
    ext: String,
    duration: f64,
}

#[derive(Clone, Serialize)]
pub struct Playback {
    pub id: Uuid,
    #[serde(skip)]
    pub poster: Option<(u64, bytes::Bytes)>,
    clip: Clip,
    started: Option<u64>,
}

pub fn routes() -> Router<App> {
    Router::new()
        .route("/api/dances", get(list))
        .route("/api/dances/import/{name}", put(import))
        .route("/api/dances/{id}", post(play).delete(remove))
        .route("/api/dances/stop/{id}", post(stop))
        .route("/api/stage/dance/{id}", post(report))
        .route("/dance-video/{id}", get(video))
}

fn catalog(app: &App) -> Result<Vec<Clip>> {
    let path = app.dir.join("dances.json");
    if path.exists() {
        config::read_json(&path)
    } else {
        Ok(Vec::new())
    }
}
fn media_path(app: &App, clip: &Clip) -> PathBuf {
    app.dir
        .join("dances")
        .join(format!("{}.{}", clip.id, clip.ext))
}
async fn list(State(app): State<App>) -> Result<Json<Vec<Clip>>, Problem> {
    Ok(Json(catalog(&app)?))
}

async fn duration(path: &std::path::Path) -> Result<f64> {
    let exe = audio::ffmpeg()
        .context("请先安装 FFmpeg")?
        .with_file_name("ffprobe");
    let probe = tokio::process::Command::new(exe)
        .args([
            "-v",
            "error",
            "-protocol_whitelist",
            "file",
            "-format_whitelist",
            "mov,matroska,webm",
            "-select_streams",
            "V:0",
            "-show_entries",
            "stream=width,height:format=duration",
            "-of",
            "json",
        ])
        .arg(path)
        .kill_on_drop(true)
        .output();
    let out = tokio::time::timeout(Duration::from_secs(30), probe).await??;
    ensure!(out.status.success(), "无法读取舞蹈视频");
    let v: Value = serde_json::from_slice(&out.stdout)?;
    ensure!(
        v.pointer("/streams/0/width")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            > 0,
        "文件缺少视频轨道"
    );
    let secs: f64 = v
        .pointer("/format/duration")
        .and_then(Value::as_str)
        .context("无法读取视频时长")?
        .parse()?;
    ensure!(secs.is_finite() && secs > 0.0, "无法读取视频时长");
    Ok(secs)
}

async fn import(
    State(app): State<App>,
    Path(name): Path<String>,
    body: Body,
) -> Result<Json<Clip>, Problem> {
    Ok(Json(save(&app, &name, body).await?))
}
async fn save(app: &App, name: &str, body: Body) -> Result<Clip> {
    let (title, ext) = name
        .rsplit_once('.')
        .context("请选择 MP4、MOV 或 WebM 视频")?;
    let ext = ext.to_ascii_lowercase();
    ensure!(
        matches!(ext.as_str(), "mp4" | "mov" | "webm"),
        "请选择 MP4、MOV 或 WebM 视频"
    );
    ensure!(
        !title.trim().is_empty() && title.len() <= 512 && !title.chars().any(char::is_control),
        "视频名称无效"
    );
    let dir = app.dir.join("dances");
    config::prepare_dir(&dir)?;
    let temp = tempfile::NamedTempFile::new_in(&dir)?;
    let mut file = tokio::fs::File::from_std(temp.reopen()?);
    let mut size = 0;
    let mut stream = body.into_data_stream();
    while let Some(chunk) = stream.next().await {
        let bytes = chunk?;
        size += bytes.len();
        ensure!(size <= 1024 * 1024 * 1024, "单个舞蹈视频不能超过 1 GiB");
        file.write_all(&bytes).await?;
    }
    file.flush().await?;
    let clip = Clip {
        id: Uuid::new_v4(),
        title: title.trim().into(),
        ext,
        duration: duration(temp.path()).await?,
    };
    let _guard = app.library_gate.lock().await;
    let mut clips = catalog(app)?;
    let dest = media_path(app, &clip);
    temp.persist(&dest)?;
    clips.push(clip.clone());
    if let Err(e) = config::write_json(&app.dir.join("dances.json"), &clips) {
        let _ = std::fs::remove_file(dest);
        return Err(e);
    }
    Ok(clip)
}

async fn remove(State(app): State<App>, Path(id): Path<Uuid>) -> Result<Json<Value>, Problem> {
    let _guard = app.library_gate.lock().await;
    let mut clips = catalog(&app)?;
    let index = clips
        .iter()
        .position(|c| c.id == id)
        .context("舞蹈视频不存在")?;
    if lock(&app.screen)
        .dance
        .as_ref()
        .is_some_and(|d| d.clip.id == id)
    {
        return Err(anyhow::anyhow!("请先结束舞蹈播放").into());
    }
    let clip = clips.remove(index);
    config::write_json(&app.dir.join("dances.json"), &clips)?;
    let _ = tokio::fs::remove_file(media_path(&app, &clip)).await;
    Ok(Json(json!({"ok":true})))
}

async fn play(State(app): State<App>, Path(id): Path<Uuid>) -> Result<Json<Value>, Problem> {
    let _guard = app.library_gate.lock().await;
    let clip = catalog(&app)?
        .into_iter()
        .find(|c| c.id == id)
        .context("舞蹈视频不存在")?;
    let mut s = lock(&app.screen);
    if s.lease.is_empty() || config::now().saturating_sub(s.seen) > 10000 {
        return Err(anyhow::anyhow!("请先打开舞台").into());
    }
    s.dance = Some(Playback {
        id: Uuid::new_v4(),
        clip,
        started: None,
        poster: None,
    });
    Ok(Json(json!({"ok":true})))
}
async fn stop(State(app): State<App>, Path(id): Path<Uuid>) -> Json<Value> {
    let mut s = lock(&app.screen);
    if s.dance.as_ref().is_some_and(|d| d.id == id) {
        s.dance = None;
    }
    Json(json!({"ok":true}))
}

#[derive(Deserialize)]
struct Receipt {
    client: String,
    state: String,
}
async fn report(
    State(app): State<App>,
    Path(id): Path<Uuid>,
    Json(q): Json<Receipt>,
) -> Json<Value> {
    let mut s = lock(&app.screen);
    if s.lease == q.client
        && let Some(d) = s.dance.as_mut().filter(|d| d.id == id)
    {
        match q.state.as_str() {
            "playing" => {
                d.started.get_or_insert_with(config::now);
            }
            "ended" => s.dance = None,
            "error" => {
                let title = d.clip.title.clone();
                s.dance = None;
                drop(s);
                app.event("舞蹈视频", format!("视频播放失败 · {title}"));
            }
            _ => {}
        }
    }
    Json(json!({"ok":true}))
}
async fn video(State(app): State<App>, Path(id): Path<Uuid>, req: Request<Body>) -> Response {
    let clip = lock(&app.screen)
        .dance
        .as_ref()
        .filter(|d| d.id == id)
        .map(|d| d.clip.clone());
    let Some(clip) = clip else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match ServeFile::new(media_path(&app, &clip)).try_call(req).await {
        Ok(response) => response.into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}
