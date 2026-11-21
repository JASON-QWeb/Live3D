mod api;
mod audience;
mod audio;
mod choreo;
mod config;
mod dialogue;
mod director;
mod library;
mod obs;
mod output;
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
use std::{io::ErrorKind, time::Duration};
use tokio::net::TcpListener;

async fn open(admin: &str, port: u16) -> Result<()> {
    let url = format!("http://127.0.0.1:{port}/#{admin}");
    let status = tokio::process::Command::new("/usr/bin/open")
        .arg(url)
        .status()
        .await?;
    ensure!(status.success(), "macOS 未能打开控制台浏览器");
    Ok(())
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
#[tokio::main]
async fn main() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.iter().any(|s| s == "--version") {
        println!("Avatar3D Live {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let dir = config::data_dir()?;
    let port = std::env::var("AVATAR3D_PORT")
        .ok()
        .map(|s| s.parse::<u16>())
        .transpose()?
        .unwrap_or(config::PORT);
    let headless = args.iter().any(|a| a == "--no-open");
    let listener = match TcpListener::bind(("127.0.0.1", port)).await {
        Ok(l) => l,
        Err(e) if e.kind() == ErrorKind::AddrInUse && !headless => {
            let http = reqwest::Client::new();
            let r = http
                .get(format!("http://127.0.0.1:{port}/health"))
                .timeout(Duration::from_secs(1))
                .send()
                .await?;
            if r.text().await? == "avatar3d-live" && dir.join("access.json").is_file() {
                return open(&config::Tokens::load(&dir)?.admin, port).await;
            }
            bail!("端口 {port} 已被其他程序占用")
        }
        Err(e) => return Err(e.into()),
    };
    config::prepare_dir(&dir)?;
    let instance = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("studio.lock"))?;
    instance
        .try_lock()
        .context("同一数据目录已有 Avatar3D 在运行")?;
    let app = Inner::new(dir)?;
    if !headless {
        open(&app.tokens.admin, port)
            .await
            .context("无法打开默认浏览器")?;
    }
    tokio::spawn(prompt_store::watch(app.clone()));
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
