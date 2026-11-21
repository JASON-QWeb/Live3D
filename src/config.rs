use anyhow::{Context, Result, anyhow, bail, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    env, fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};
use uuid::Uuid;

pub const PORT: u16 = 19848;
const SERVICE: &str = "studio.avatar3d.core";
pub const KEY_NAMES: [&str; 3] = ["vidu", "llm", "youtube"];

#[derive(Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub avatar_id: String,
    pub action_library: String,
    pub resolution: String,
    pub max_minutes: u32,
    pub tts_mode: String,
    pub tts_endpoint: String,
    pub tts_model: String,
    pub voice: String,
    pub reference_audio: String,
    pub reference_text: String,
    pub llm_endpoint: String,
    pub llm_model: String,
    pub persona: String,
    pub youtube_video: String,
    pub audience_mode: String,
    pub subtitles: bool,
    pub recording: bool,
    pub actions: Vec<String>,
    pub action_gap: u32,
    pub song_requests: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            avatar_id: String::new(),
            action_library: String::new(),
            resolution: "720p".into(),
            max_minutes: 5,
            tts_mode: "mlx".into(),
            tts_endpoint: "http://127.0.0.1:19849/v1/audio/speech".into(),
            tts_model: "mlx-community/Kokoro-82M-bf16".into(),
            voice: "zf_xiaobei".into(),
            reference_audio: String::new(),
            reference_text: String::new(),
            llm_endpoint: "http://127.0.0.1:19847/v1/chat/completions".into(),
            llm_model: "mlx-community/Qwen2.5-1.5B-Instruct-4bit".into(),
            persona: "用简短自然的中文回应直播观众，输出适合朗读的正文。涉及价格、库存或现场情况时，仅依据提供的信息。".into(),
            youtube_video: String::new(),
            audience_mode: "review".into(),
            subtitles: true,
            recording: false,
            actions: Vec::new(),
            action_gap: 12,
            song_requests: false,
        }
    }
}
fn check_url(value: &str) -> Result<()> {
    let u = url::Url::parse(value)?;
    ensure!(
        u.username().is_empty()
            && u.password().is_none()
            && u.query().is_none()
            && u.fragment().is_none(),
        "服务地址不能包含密钥、查询参数或片段"
    );
    let local = matches!(u.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
    ensure!(
        u.scheme() == "https" || (local && u.scheme() == "http"),
        "服务地址须为 HTTPS 或本机 HTTP"
    );
    Ok(())
}
impl Settings {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (1..=1440).contains(&self.max_minutes),
            "连线时限为 1 至 1440 分钟"
        );
        ensure!(
            ["720p", "1080p", "2k"].contains(&self.resolution.as_str()),
            "分辨率无效"
        );
        ensure!(
            ["mlx", "system"].contains(&self.tts_mode.as_str()),
            "语音方式无效"
        );
        ensure!(
            ["review", "read", "reply"].contains(&self.audience_mode.as_str()),
            "弹幕模式无效"
        );
        ensure!(
            (2..=300).contains(&self.action_gap)
                && self.actions.len() <= 100
                && self
                    .actions
                    .iter()
                    .all(|s| !s.trim().is_empty() && s.chars().count() <= 500),
            "编排最多 100 条非空动作，间隔 2 至 300 秒"
        );
        if !self.llm_endpoint.is_empty() {
            check_url(&self.llm_endpoint)?;
        }
        if self.tts_mode == "mlx" {
            check_url(&self.tts_endpoint)?;
            ensure!(!self.tts_model.trim().is_empty(), "请填写语音模型名称");
        }
        ensure!(
            self.youtube_video.is_empty()
                || (self.youtube_video.len() == 11
                    && self
                        .youtube_video
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')),
            "请填写 11 位 YouTube 直播视频 ID"
        );
        ensure!(
            self.persona.len() <= 16_000 && self.reference_text.len() <= 4000,
            "文本过长"
        );
        ensure!(
            self.reference_audio.is_empty() == self.reference_text.trim().is_empty(),
            "参考录音和逐字稿须一同填写"
        );
        Ok(())
    }
}
pub fn data_dir() -> Result<PathBuf> {
    if let Some(p) = env::var_os("AVATAR3D_DATA_DIR") {
        return Ok(p.into());
    }
    Ok(
        PathBuf::from(env::var_os("HOME").context("无法定位用户目录")?)
            .join("Library/Application Support/Avatar3D Live"),
    )
}
pub fn prepare_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}
pub fn read_json<T: DeserializeOwned>(p: &Path) -> Result<T> {
    serde_json::from_slice(&fs::read(p)?).with_context(|| format!("无法读取 {}", p.display()))
}
pub fn atomic(p: &Path, data: &[u8]) -> Result<()> {
    let parent = p.parent().context("无法定位配置目录")?;
    let mut f = tempfile::NamedTempFile::new_in(parent)?;
    f.write_all(data)?;
    f.as_file().sync_all()?;
    f.persist(p)?;
    Ok(())
}
pub fn write_json<T: Serialize>(p: &Path, value: &T) -> Result<()> {
    atomic(p, &serde_json::to_vec_pretty(value)?)
}
pub fn load_settings(dir: &Path) -> Result<Settings> {
    let p = dir.join("settings.json");
    let conf: Settings = if p.exists() {
        read_json(&p)?
    } else {
        Settings::default()
    };
    conf.validate()?;
    Ok(conf)
}
#[derive(Serialize, Deserialize)]
pub struct Tokens {
    pub admin: String,
    pub stage: String,
    pub ingest: String,
}
impl Tokens {
    pub fn load(dir: &Path) -> Result<Self> {
        let p = dir.join("access.json");
        if p.exists() {
            return read_json(&p);
        }
        let token = || format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let t = Self {
            admin: token(),
            stage: token(),
            ingest: token(),
        };
        write_json(&p, &t)?;
        Ok(t)
    }
}
pub fn key(name: &str) -> Result<Option<String>> {
    ensure!(KEY_NAMES.contains(&name), "未知密钥");
    if let Ok(v) = env::var(format!("AVATAR3D_{}_KEY", name.to_ascii_uppercase()))
        && !v.trim().is_empty()
    {
        return Ok(Some(v.trim().to_owned()));
    }
    match security_framework::passwords::get_generic_password(SERVICE, name) {
        Ok(v) => Ok(Some(String::from_utf8(v)?)),
        Err(e) if e.code() == -25300 => Ok(None),
        Err(e) => Err(anyhow!("无法读取 macOS 钥匙串 ({})", e.code())),
    }
}
pub fn store_key(name: &str, value: &str) -> Result<()> {
    ensure!(KEY_NAMES.contains(&name), "未知密钥");
    let v = value.trim();
    if v.is_empty() {
        match security_framework::passwords::delete_generic_password(SERVICE, name) {
            Err(e) if e.code() != -25300 => return Err(e.into()),
            _ => (),
        }
    } else {
        ensure!(
            v.len() < 8192 && !v.chars().any(char::is_control),
            "密钥格式无效"
        );
        security_framework::passwords::set_generic_password(SERVICE, name, v.as_bytes())?;
    }
    Ok(())
}
pub fn image_type(data: &[u8]) -> Result<&'static str> {
    ensure!(
        !data.is_empty() && data.len() <= 8 * 1024 * 1024,
        "图片须在 8 MiB 以内"
    );
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Ok("image/png");
    }
    if data.starts_with(&[255, 216, 255]) {
        return Ok("image/jpeg");
    }
    if data.starts_with(b"RIFF") && data.get(8..12) == Some(b"WEBP") {
        return Ok("image/webp");
    }
    bail!("支持 PNG、JPEG 和 WebP 图片")
}
pub fn image_uri(data: &[u8]) -> Result<String> {
    Ok(format!(
        "data:{};base64,{}",
        image_type(data)?,
        STANDARD.encode(data)
    ))
}
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_settings_gain_defaults_for_new_controls() {
        let mut old = serde_json::to_value(Settings::default()).unwrap();
        let fields = old.as_object_mut().unwrap();
        for name in ["actions", "action_gap", "song_requests"] {
            fields.remove(name);
        }
        let loaded: Settings = serde_json::from_value(old).unwrap();
        loaded.validate().unwrap();
        assert!(loaded.actions.is_empty());
        assert!(!loaded.song_requests);
        assert_eq!(loaded.action_gap, Settings::default().action_gap);
    }
}
