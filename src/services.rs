use crate::{
    audio::{checked, limited},
    config,
    state::App,
};
use anyhow::{Context, Result, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha384};
use std::{fs, path::PathBuf, process::Stdio, time::Duration};

const SDK_HASH: &str = "ly1Gkjo/1LyiaAdx8mfZ4N/BpBIgFNVQK4FvKp25yli3N1IZPzfrEXSU/7ijlVfr";
const SDK_URL: &str = "https://cdn.jsdelivr.net/npm/trtc-sdk-v5@5.21.0/trtc.js";
pub async fn sdk(app: &App) -> Result<Vec<u8>> {
    let _g = app.sdk_gate.lock().await;
    let path = app.dir.join("trtc-5.21.0.js");
    let valid = |bytes: &[u8]| STANDARD.encode(Sha384::digest(bytes)) == SDK_HASH;
    if let Ok(b) = fs::read(&path)
        && valid(&b)
    {
        return Ok(b);
    }
    let r = app
        .http
        .get(SDK_URL)
        .timeout(Duration::from_secs(25))
        .send()
        .await
        .map_err(|e| e.without_url())?;
    let b = limited(checked(r, "RTC SDK 下载").await?, 4_000_000).await?;
    ensure!(valid(&b), "RTC SDK 完整性校验未通过");
    config::atomic(&path, &b)?;
    Ok(b)
}

#[derive(Clone, Copy)]
pub enum ModelService {
    Voice,
    Chat,
}
impl ModelService {
    fn name(self) -> &'static str {
        match self {
            Self::Voice => "voice",
            Self::Chat => "chat",
        }
    }
    fn process(self, app: &App) -> &tokio::sync::Mutex<Option<tokio::process::Child>> {
        match self {
            Self::Voice => &app.voice_process,
            Self::Chat => &app.chat_process,
        }
    }
    async fn ready(self, app: &App) -> bool {
        let addr = match self {
            Self::Voice => "http://127.0.0.1:19849/openapi.json",
            Self::Chat => "http://127.0.0.1:19847/v1/models",
        };
        let Ok(r) = app
            .http
            .get(addr)
            .timeout(Duration::from_secs(2))
            .send()
            .await
        else {
            return false;
        };
        let Ok(b) = limited(r, 2_000_000).await else {
            return false;
        };
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(&b) else {
            return false;
        };
        match self {
            Self::Voice => v["paths"].get("/v1/audio/speech").is_some(),
            Self::Chat => v["data"].is_array(),
        }
    }
    pub async fn launch(self, app: &App) -> Result<&'static str> {
        let mut process = self.process(app).lock().await;
        if self.ready(app).await {
            return Ok("本机服务已就绪");
        }
        if let Some(c) = process.as_mut()
            && c.try_wait()?.is_none()
        {
            return Ok("本机服务正在启动");
        }
        let dir = std::env::var_os(format!("AVATAR3D_{}_DIR", self.name().to_uppercase()))
            .map(PathBuf::from)
            .unwrap_or_else(|| app.dir.join(self.name()));
        config::prepare_dir(&dir)?;
        let log = fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(app.dir.join(format!("{}.log", self.name())))?;
        let c = tokio::process::Command::new("/bin/bash")
            .args([
                "-c",
                include_str!("../scripts/start-model.sh"),
                "avatar3d",
                self.name(),
            ])
            .current_dir(&dir)
            .env("AVATAR3D_MODEL_DIR", &dir)
            .env("AVATAR3D_MODEL_CACHE", app.dir.join("models"))
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log)
            .kill_on_drop(true)
            .spawn()
            .context("无法启动本机模型服务")?;
        *process = Some(c);
        Ok("正在启动本机模型，首次使用会下载依赖与权重")
    }
    pub async fn status(self, app: &App) -> String {
        let mut pro = self.process(app).lock().await;
        if let Some(c) = pro.as_mut()
            && let Ok(Some(s)) = c.try_wait()
        {
            return format!("进程已退出（{s}）。详见数据目录中的 {}.log", self.name());
        }
        if self.ready(app).await {
            return "已就绪".into();
        }
        if pro.is_some() {
            "启动中".into()
        } else {
            "未启动".into()
        }
    }
}
