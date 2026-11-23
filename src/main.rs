mod api;
mod audience;
mod audio;
mod choreo;
mod config;
mod desktop;
mod dialogue;
mod director;
mod douyin;
mod library;
mod obs;
mod output;
mod paths;
mod prompt_store;
mod prompts;
mod replies;
mod server;
mod services;
mod session;
mod soundcheck;
mod speech;
mod state;
mod vidu;
mod wire;

use anyhow::{Context, Result, bail, ensure};
use state::{App, Inner, lock};
use std::{fs::File, io::ErrorKind, path::Path, time::Duration};
use tokio::net::TcpListener;

async fn attach(dir: &Path, port: u16) -> Result<()> {
    let http = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(2))
        .build()?;
    let health = http
        .get(format!("http://127.0.0.1:{port}/health"))
        .send()
        .await?;
    let same_version = health
        .headers()
        .get("x-avatar3d-version")
        .is_some_and(|value| value.as_bytes() == env!("CARGO_PKG_VERSION").as_bytes());
    ensure!(
        health.text().await? == "avatar3d-live",
        "端口 {port} 已被其他程序占用"
    );
    ensure!(
        same_version,
        "另一版本的 Avatar3D 正在运行。请在方便结束当前任务时，在原控制台的「设置」点「退出 Avatar3D」，然后重新打开此应用。"
    );
    let instance =
        File::open(dir.join("studio.lock")).context("该端口上的 Avatar3D 使用其他数据目录")?;
    match instance.try_lock() {
        Err(std::fs::TryLockError::WouldBlock) => (),
        Ok(()) => bail!("该端口上的 Avatar3D 使用其他数据目录"),
        Err(e) => return Err(e.into()),
    }
    let tokens: config::Tokens = config::read_json(&dir.join("access.json"))?;
    // An attached window closes when its existing service releases the data directory.
    std::thread::spawn(move || {
        if instance.lock().is_ok() {
            desktop::service_ended();
        }
    });
    desktop::show(port, &tokens.admin, None)
}
async fn shutdown(app: App) {
    let mut term =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("SIGTERM");
    tokio::select! {
        _ = app.quit.cancelled() => (),
        _ = tokio::signal::ctrl_c() => (),
        _ = term.recv() => (),
    }
    app.quit.cancel();
    if lock(&app.run).is_some() {
        let _ = session::stop(&app);
    }
    while lock(&app.run).is_some() {
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}

async fn lock_directory(dir: &Path) -> Result<File> {
    let file = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("studio.lock"))?;
    // A relaunch can bind the port just before the previous server finishes cleaning up.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(std::fs::TryLockError::WouldBlock) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Err(error) => return Err(error).context("同一数据目录已有 Avatar3D 在运行"),
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.iter().any(|s| s == "--version") {
        println!("Avatar3D Live {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let headless = args.iter().any(|a| a == "--no-open");
    let result = run(headless).await;
    if !headless && let Err(error) = &result {
        desktop::startup_error(error);
    }
    result
}

async fn run(headless: bool) -> Result<()> {
    let dir = config::data_dir()?;
    let port = std::env::var("AVATAR3D_PORT")
        .ok()
        .map(|s| s.parse::<u16>())
        .transpose()?
        .unwrap_or(config::PORT);
    let listener = match TcpListener::bind(("127.0.0.1", port)).await {
        Ok(l) => l,
        Err(e) if e.kind() == ErrorKind::AddrInUse && !headless => {
            return attach(&dir, port).await;
        }
        Err(e) => return Err(e.into()),
    };
    config::prepare_dir(&dir)?;
    let instance = lock_directory(&dir).await?;
    paths::prepare_bundled_avatars(&dir)?;
    let port = listener.local_addr()?.port();
    let app = Inner::new(dir)?;
    if headless {
        return serve(listener, app, port, instance).await;
    }
    let core = tokio::spawn(serve(listener, app.clone(), port, instance));
    let finished = tokio::spawn(async move {
        let result = core.await.context("控制台服务意外终止").and_then(|r| r);
        desktop::service_ended();
        result
    });
    let window = desktop::show(port, &app.tokens.admin, Some(&app.quit));
    app.quit.cancel();
    finished.await??;
    window
}

async fn serve(listener: TcpListener, app: App, port: u16, _directory_lock: File) -> Result<()> {
    tokio::spawn(prompt_store::watch(app.clone()));
    tokio::spawn(douyin::run(app.clone()));
    println!("Avatar3D 正在 127.0.0.1:{port} 运行");
    axum::serve(listener, server::router(app.clone(), port))
        .with_graceful_shutdown(shutdown(app.clone()))
        .await?;
    if let Some(mut proc) = app.chat_process.lock().await.take() {
        let _ = proc.kill().await;
    }
    if let Some(mut proc) = app.voice_process.lock().await.take() {
        let _ = proc.kill().await;
    }
    Ok(())
}
