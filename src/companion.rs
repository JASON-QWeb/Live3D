//! Owns the macOS reader. Only confirmed rows in forwarding mode reach the
//! same authenticated HTTP endpoint that external collectors use.
use crate::state::{App, lock};
use axum::{Json, Router, extract::State, http::StatusCode, response::IntoResponse, routing::get};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    path::PathBuf,
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::{Child, Command},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
enum Mode {
    #[default]
    Off,
    Preview,
    Forward,
}

#[derive(Deserialize)]
struct Change {
    mode: Mode,
}

#[derive(Deserialize)]
struct Line {
    kind: String,
    text: String,
    author: Option<String>,
    gift_name: Option<String>,
    gift_count: Option<u32>,
}

#[derive(Clone, Serialize)]
struct Row {
    kind: String,
    detail: String,
    result: String,
}

#[derive(Clone, Default, Serialize)]
struct View {
    mode: Mode,
    status: String,
    rows: VecDeque<Row>,
}

struct Worker {
    stop: CancellationToken,
    task: JoinHandle<()>,
}

struct Controller {
    app: App,
    port: u16,
    view: Mutex<View>,
    worker: tokio::sync::Mutex<Option<Worker>>,
}

type ControllerState = State<Arc<Controller>>;

pub fn routes(app: App, port: u16) -> Router<App> {
    let ctl = Arc::new(Controller {
        app,
        port,
        view: Mutex::default(),
        worker: tokio::sync::Mutex::default(),
    });
    Router::new()
        .route("/api/douyin-companion", get(status).post(change))
        .with_state(ctl)
}

async fn status(State(ctl): ControllerState) -> Json<View> {
    Json(lock(&ctl.view).clone())
}

fn executable() -> anyhow::Result<PathBuf> {
    if let Some(contents) = crate::paths::bundle_contents() {
        let helper = contents.join("Helpers/avatar3d-douyin");
        anyhow::ensure!(helper.is_file(), "应用包中缺少直播伴侣读取组件，请重新构建");
        return Ok(helper);
    }
    let current = std::env::current_exe()?;
    let sibling = current.with_file_name("avatar3d-douyin");
    if sibling.is_file() {
        return Ok(sibling);
    }
    if current
        .parent()
        .and_then(|p| p.file_name())
        .is_none_or(|name| name != "Helpers")
        && let Some(built) = option_env!("AVATAR3D_COMPILED_COMPANION").map(PathBuf::from)
        && built.is_file()
    {
        return Ok(built);
    }
    anyhow::bail!("找不到 macOS 直播伴侣读取组件，请重新构建 Avatar3D Live")
}

async fn change(State(ctl): ControllerState, Json(request): Json<Change>) -> impl IntoResponse {
    match set_mode(&ctl, request.mode).await {
        Ok(()) => Json(lock(&ctl.view).clone()).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn set_mode(ctl: &Arc<Controller>, mode: Mode) -> anyhow::Result<()> {
    let mut worker = ctl.worker.lock().await;
    if lock(&ctl.view).mode == mode && worker.as_ref().is_some_and(|w| !w.task.is_finished()) {
        return Ok(());
    }
    if let Some(old) = worker.take() {
        old.stop.cancel();
        let _ = old.task.await;
    }
    {
        let mut view = lock(&ctl.view);
        view.mode = Mode::Off;
        view.status.clear();
    }
    if mode == Mode::Off {
        return Ok(());
    }
    let http = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(2))
        .build()?;
    let child = Command::new(executable()?)
        .arg("--request-access")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    {
        let mut view = lock(&ctl.view);
        view.mode = mode;
        view.status = "正在连接本机直播伴侣".into();
    }
    let stop = CancellationToken::new();
    *worker = Some(Worker {
        stop: stop.clone(),
        task: tokio::spawn(run(ctl.clone(), child, mode, stop, http)),
    });
    Ok(())
}

async fn run(
    ctl: Arc<Controller>,
    mut child: Child,
    mode: Mode,
    stop: CancellationToken,
    http: reqwest::Client,
) {
    let mut lines = BufReader::new(child.stdout.take().expect("piped reader output")).lines();
    let reason = loop {
        tokio::select! {
            biased;
            _ = stop.cancelled() => break None,
            _ = ctl.app.quit.cancelled() => break None,
            result = lines.next_line() => match result {
                Ok(Some(line)) => match serde_json::from_str::<Line>(&line) {
                    Ok(line) => ctl.handle(line, mode, &http).await,
                    Err(_) => break Some("读取组件返回了无法识别的数据"),
                },
                Ok(None) | Err(_) => break Some("读取组件已退出，可重新开启"),
            }
        }
    };
    drop(child.stdin.take());
    if tokio::time::timeout(Duration::from_secs(2), child.wait())
        .await
        .is_err()
    {
        let _ = child.kill().await;
    }
    let mut view = lock(&ctl.view);
    view.mode = Mode::Off;
    view.status = reason.unwrap_or_default().into();
}

impl Controller {
    async fn handle(&self, line: Line, mode: Mode, http: &reqwest::Client) {
        if line.kind == "status" {
            lock(&self.view).status = line.text;
            return;
        }
        let author = line.author.as_deref().unwrap_or("").trim();
        let name = line.gift_name.as_deref().unwrap_or("").trim();
        let (label, detail, confirmed) = match line.kind.as_str() {
            "chat" if !author.is_empty() && !line.text.is_empty() => {
                ("弹幕", format!("{author} · {}", line.text), true)
            }
            "gift"
                if !author.is_empty()
                    && !name.is_empty()
                    && line.gift_count.is_some_and(|n| n > 0) =>
            {
                (
                    "礼物",
                    format!("{author} · {name} ×{}", line.gift_count.unwrap()),
                    true,
                )
            }
            _ => ("待核对", line.text.clone(), false),
        };
        let result = if !confirmed {
            "仅供核对".into()
        } else if mode == Mode::Preview {
            "仅预览".into()
        } else {
            match self.forward(http, &line).await {
                Ok(()) => "已接入".into(),
                Err(e) => format!("接入失败 · {e}"),
            }
        };
        let mut view = lock(&self.view);
        view.rows.push_front(Row {
            kind: label.into(),
            detail,
            result,
        });
        view.rows.truncate(30);
    }

    async fn forward(&self, http: &reqwest::Client, line: &Line) -> anyhow::Result<()> {
        let url = format!("http://127.0.0.1:{}/api/ingest", self.port);
        let body = json!({
            "platform": "douyin", "kind": line.kind, "author": line.author, "text": line.text,
            "id": format!("dy-companion/{}", uuid::Uuid::new_v4()),
            "gift_name": line.gift_name, "gift_count": line.gift_count,
        });
        let send = || {
            http.post(&url)
                .bearer_auth(&self.app.tokens.ingest)
                .json(&body)
                .send()
        };
        let response = match send().await {
            Ok(r) => r,
            Err(_) => send().await?,
        };
        let status = response.status();
        let result: Value = response.json().await?;
        anyhow::ensure!(
            status.is_success() && result["accepted"].is_boolean(),
            "{}",
            result["error"].as_str().unwrap_or("本机弹幕入口未确认")
        );
        Ok(())
    }
}
