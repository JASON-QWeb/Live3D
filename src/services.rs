use crate::{
    audio::{checked, limited},
    config,
    state::App,
};
use anyhow::{Context, Result, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use sha2::{Digest, Sha384};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

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
    fn dir(self, app: &App) -> PathBuf {
        std::env::var_os(format!("VORELA_{}_DIR", self.name().to_uppercase()))
            .map(PathBuf::from)
            .unwrap_or_else(|| app.dir.join(self.name()))
    }
    fn log(self, app: &App) -> PathBuf {
        app.dir.join(format!("{}.log", self.name()))
    }
    async fn voice_loaded_language(app: &App) -> Option<String> {
        let r = app
            .http
            .get("http://127.0.0.1:19849/v1/models")
            .timeout(Duration::from_secs(2))
            .send()
            .await
            .ok()?;
        let b = limited(r, 2_000_000).await.ok()?;
        let v = serde_json::from_slice::<serde_json::Value>(&b).ok()?;
        if let Some(lang) = v["language"].as_str() {
            return Some(lang.to_owned());
        }
        let ids = v["data"]
            .as_array()?
            .iter()
            .filter_map(|item| item["id"].as_str())
            .collect::<Vec<_>>();
        if ids.len() == 1 && ids[0] == config::ZONOS {
            return Some("en".into());
        }
        if ids.len() == 1 && ids[0] == config::MING {
            return Some("zh".into());
        }
        Some("both".into())
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
            Self::Voice => {
                v["info"]["title"] == "Vorela Voice"
                    && v["paths"].get("/v1/audio/speech").is_some()
            }
            Self::Chat => v["data"].is_array(),
        }
    }
    pub async fn stop(self, app: &App) {
        let mut process = self.process(app).lock().await;
        if let Some(mut c) = process.take() {
            let _ = c.kill().await;
            let _ = c.wait().await;
        }
    }
    pub async fn launch(self, app: &App) -> Result<&'static str> {
        let mut process = self.process(app).lock().await;
        let desired_lang = app.config().live_language;
        if self.ready(app).await {
            if matches!(self, Self::Voice) {
                let loaded = Self::voice_loaded_language(app).await;
                if loaded.as_deref() == Some(desired_lang.as_str()) {
                    return Ok("本机服务已就绪");
                }
                if let Some(mut old) = process.take() {
                    let _ = old.kill().await;
                    let _ = old.wait().await;
                } else {
                    anyhow::bail!(
                        "本机语音端口 19849 正运行另一语言实例，请先结束旧进程后再启动"
                    );
                }
            } else {
                return Ok("本机服务已就绪");
            }
        }
        if let Some(c) = process.as_mut()
            && c.try_wait()?.is_none()
        {
            return Ok("本机服务正在启动");
        }
        let dir = self.dir(app);
        config::prepare_dir(&dir)?;
        config::atomic(
            &dir.join("model_fetch.py"),
            include_bytes!("../scripts/model_fetch.py"),
        )?;
        if matches!(self, Self::Voice) {
            config::atomic(
                &dir.join("voice-service.py"),
                include_bytes!("../scripts/voice-service.py"),
            )?;
        }
        match fs::remove_file(dir.join("progress.json")) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
            _ => (),
        }
        let log = fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(self.log(app))?;
        let mut cmd = tokio::process::Command::new("/bin/bash");
        cmd.args([
            "-c",
            include_str!("../scripts/start-model.sh"),
            "vorela",
            self.name(),
        ])
        .current_dir(&dir)
        .env("VORELA_MODEL_DIR", &dir)
        .env("VORELA_MODEL_CACHE", app.dir.join("models"))
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        .kill_on_drop(true);
        if matches!(self, Self::Voice) {
            cmd.env("VORELA_VOICE_LANG", &desired_lang);
        }
        let c = cmd.spawn().context("无法启动本机模型服务")?;
        *process = Some(c);
        Ok("正在启动本机模型，首次使用会下载依赖与权重")
    }
    /// `{"message"}` for the settings banner, plus `{"progress"}` while the service starts.
    pub async fn status(self, app: &App) -> Value {
        let mut pro = self.process(app).lock().await;
        if let Some(c) = pro.as_mut()
            && let Ok(Some(s)) = c.try_wait()
        {
            let message = format!("进程已退出（{s}）。详见数据目录中的 {}.log", self.name());
            return json!({"message":message});
        }
        if self.ready(app).await {
            if matches!(self, Self::Voice) {
                let desired = app.config().live_language;
                let loaded = Self::voice_loaded_language(app).await;
                let message = match loaded.as_deref() {
                    Some(l) if l == desired && l == "en" => "已就绪（英文 · Willow）",
                    Some(l) if l == desired => "已就绪（中文 · 灵小柔）",
                    Some("en") => "当前为英文语音，与所选中文环境不一致，请重启本机 MLX",
                    Some("zh") => "当前为中文语音，与所选英文环境不一致，请重启本机 MLX",
                    _ => "当前为旧版双语实例，请重启本机 MLX 切换为单语模式",
                };
                return json!({"message":message});
            }
            return json!({"message":"已就绪"});
        }
        if pro.is_some() {
            let progress = startup_progress(&self.dir(app), &self.log(app));
            json!({"message":"启动中","progress":progress})
        } else {
            json!({"message":"未启动"})
        }
    }
}

/// The startup step that start-model.sh and model_fetch.py last recorded in `dir`.
fn startup_progress(dir: &Path, log: &Path) -> Option<Value> {
    let b = fs::read(dir.join("progress.json")).ok()?;
    let p = serde_json::from_slice::<Value>(&b).ok()?;
    let stage = p["stage"].as_str()?;
    let (done, total) = match stage {
        "env" | "load" => (0, 0),
        "packages" => fs::read(log)
            .ok()
            .and_then(|log| package_progress(&String::from_utf8_lossy(&log)))
            .unwrap_or((0, 0)),
        "download" => (
            p["done"].as_u64().unwrap_or(0),
            p["total"].as_u64().unwrap_or(0),
        ),
        _ => return None,
    };
    Some(json!({"stage":stage,"done":done,"total":total}))
}

/// Bytes of the larger packages uv has downloaded, out of those it announced.
/// uv only names packages above its size threshold, so smaller ones are not counted.
fn package_progress(log: &str) -> Option<(u64, u64)> {
    let mut sizes = HashMap::new();
    let mut fetched = HashSet::new();
    for line in log.lines().map(str::trim) {
        if let Some((name, size)) = line
            .strip_prefix("Downloading ")
            .and_then(|rest| rest.split_once(" ("))
            && let Some(bytes) = size.strip_suffix(')').and_then(byte_size)
        {
            sizes.insert(name, bytes);
        } else if let Some(name) = line.strip_prefix("Downloaded ") {
            fetched.insert(name);
        }
    }
    let total = sizes.values().sum::<u64>();
    let done = sizes
        .iter()
        .filter(|(name, _)| fetched.contains(*name))
        .map(|(_, bytes)| bytes)
        .sum();
    (total > 0).then_some((done, total))
}

fn byte_size(s: &str) -> Option<u64> {
    let (number, unit) = s.split_at(s.find(|c: char| c.is_ascii_alphabetic())?);
    let scale = match unit {
        "B" => 1.0,
        "KiB" => 1024.0,
        "MiB" => 1024.0 * 1024.0,
        "GiB" => 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };
    Some((number.parse::<f64>().ok()? * scale) as u64)
}

#[cfg(test)]
mod tests {
    use super::{package_progress, startup_progress};
    use serde_json::json;

    #[test]
    fn reads_startup_step_from_progress_file() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("chat.log");
        let write = |s: &str| std::fs::write(dir.path().join("progress.json"), s).unwrap();
        assert!(startup_progress(dir.path(), &log).is_none());

        write(r#"{"stage":"download","model":"m","done":5,"total":20}"#);
        assert_eq!(
            startup_progress(dir.path(), &log),
            Some(json!({"stage":"download","done":5,"total":20}))
        );
        write("{\"stage\":\"packages\"}\n");
        std::fs::write(&log, "Downloading mlx (2.0MiB)\n Downloaded mlx\n").unwrap();
        assert_eq!(
            startup_progress(dir.path(), &log),
            Some(json!({"stage":"packages","done":2_097_152,"total":2_097_152}))
        );
        write(r#"{"stage":"load","done":9}"#);
        assert_eq!(
            startup_progress(dir.path(), &log),
            Some(json!({"stage":"load","done":0,"total":0}))
        );
        write(r#"{"stage":"other"}"#);
        assert!(startup_progress(dir.path(), &log).is_none());
        write(r#"{"stage":"dow"#);
        assert!(startup_progress(dir.path(), &log).is_none());
    }

    #[test]
    fn counts_uv_downloads_that_finished() {
        let log = "Using Python 3.12.13 environment at: venv\n\
            Resolved 65 packages in 17.67s\n\
            Downloading cpython-3.12.13-macos-aarch64-none (download) (16.0MiB)\n   \
            Building jieba==0.42.1\n\
            Downloading mlx-metal (42.9MiB)\n\
            Downloading pygments (1.2MiB)\n\
            Downloading uvloop (512.0KiB)\n \
            Downloaded pygments\n \
            Downloaded uvloop\n";
        let mib = 1024 * 1024;
        let (done, total) = package_progress(log).unwrap();
        // The interpreter line from `uv venv` is not a package and does not parse.
        assert_eq!(total, 42_900 * mib / 1000 + 1_200 * mib / 1000 + mib / 2);
        assert_eq!(done, 1_200 * mib / 1000 + mib / 2);
        assert!(package_progress("Resolved 3 packages in 1s\n").is_none());
    }
}
