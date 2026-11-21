use anyhow::{Context, Result, bail, ensure};
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
pub const KEY_NAMES: [&str; 3] = ["vidu", "llm", "youtube"];
const LOCAL_TTS: &str = "http://127.0.0.1:19849/v1/audio/speech";
pub const MING: &str = "jorgepb96/Ming-3.0-tts-0.5B-4bit";
pub const ZONOS: &str = "mlx-community/ZONOS3-Mini";
pub const MING_VOICE: &str = "灵小柔";
pub const ZONOS_VOICE: &str = "willow";
const PREVIOUS_PERSONA: &str = "用简短自然的中文回应直播观众，输出适合朗读的正文。涉及价格、库存或现场情况时，仅依据提供的信息。";
const DEFAULT_PERSONA: &str = "你是深夜录音房里的温柔陪聊兼唱歌主播灵小柔，性格亲切自然。用简短口语化的方式回应直播观众，输出直接适合朗读的正文。涉及价格、库存或现场情况时，仅依据提供的信息。";
const FALLBACK_QA_ZH: &str = "\
Q：有新观众进入直播间，比如提示某位用户进入直播间时怎么回应？
A：直接自然叫出名字打个招呼就行，比如：欢迎某某，欢迎某某进入直播间。

Q：系统提示某某进入直播间时，如果正在聊天或人数较多怎么自然欢迎？
A：偶尔顺嘴说一句“欢迎某某”即可，简洁自然，不要每条进房提示都机械重复。

Q：观众说这首歌好好听、唱得真好听怎么回应？
A：谢谢，觉得好听可以点个关注。

Q：观众送了小心心、点赞、玫瑰、灯牌等小礼物怎么感谢？
A：谢谢宝子送的礼物，谢谢老板。

Q：观众送了跑车、火箭、嘉年华、城堡等贵重礼物或连续刷礼物怎么感谢？
A：感谢老板送的大礼！";

fn default_qa_zh() -> String {
    if let Some(root) = find_project_root() {
        let path = root.join("avatars/cn/xiaorou_knowledge_base.txt");
        if let Ok(text) = fs::read_to_string(&path) {
            let trimmed = text.trim();
            if !trimmed.is_empty() && trimmed.len() <= 32_000 {
                return trimmed.to_owned();
            }
        }
    }
    FALLBACK_QA_ZH.into()
}
const DEFAULT_QA_EN: &str = "\
Q: How should I greet a viewer who just joined the livestream?
A: Greet them warmly and naturally by name in one short sentence, like welcoming them in to relax and listen to some songs.

Q: What should I say during a quiet moment when I am not singing?
A: Welcome everyone to the livestream naturally and invite new viewers to say hi or relax with some music.

Q: How should I thank viewers for gifts?
A: Thank them warmly and sincerely in one or two short sentences. For bigger gifts, sound extra excited and appreciative.

Q: What should I say right after finishing a song?
A: Thank everyone for listening to the song, and if anyone sent gifts while singing, thank them warmly right after.";

fn default_welcome_phrases_zh() -> Vec<String> {
    vec![
        "欢迎大家来到小柔直播间".into(),
        "欢迎大家进入直播间".into(),
        "欢迎大家".into(),
    ]
}

fn default_welcome_phrases_en() -> Vec<String> {
    vec![
        "Welcome everyone to the livestream!".into(),
        "Welcome in everyone, so glad you're here!".into(),
        "Welcome everyone!".into(),
    ]
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub avatar_id: String,
    pub action_library: String,
    pub resolution: String,
    pub max_minutes: u32,
    pub live_language: String,
    pub tts_mode: String,
    pub tts_endpoint: String,
    pub tts_model: String,
    pub voice: String,
    pub tts_model_en: String,
    pub voice_en: String,
    pub reference_audio: String,
    pub reference_text: String,
    pub llm_endpoint: String,
    pub llm_model: String,
    pub persona: String,
    pub qa_reference: String,
    pub qa_reference_en: String,
    pub youtube_video: String,
    pub audience_mode: String,
    pub subtitles: bool,
    pub recording: bool,
    pub actions: Vec<String>,
    pub action_gap: u32,
    pub song_requests: bool,
    pub auto_welcome: bool,
    pub ambient_welcome_gap: u32,
    pub welcome_phrases: Vec<String>,
    pub welcome_phrases_en: Vec<String>,
    pub welcome_joins: bool,
    pub join_welcome_gap: u32,
    pub auto_thank_gifts: bool,
    pub auto_sing: bool,
    pub auto_sing_gap: u32,
    pub singing_action: String,
    pub singing_action_en: String,
    pub post_sing_action: String,
    pub post_sing_action_en: String,
    pub utterance_gap: u32,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            avatar_id: String::new(),
            action_library: String::new(),
            resolution: "720p".into(),
            max_minutes: 5,
            live_language: "zh".into(),
            tts_mode: "mlx".into(),
            tts_endpoint: LOCAL_TTS.into(),
            tts_model: MING.into(),
            voice: MING_VOICE.into(),
            tts_model_en: ZONOS.into(),
            voice_en: ZONOS_VOICE.into(),
            reference_audio: String::new(),
            reference_text: String::new(),
            llm_endpoint: "http://127.0.0.1:19847/v1/chat/completions".into(),
            llm_model: "mlx-community/Qwen2.5-1.5B-Instruct-4bit".into(),
            persona: DEFAULT_PERSONA.into(),
            qa_reference: default_qa_zh(),
            qa_reference_en: DEFAULT_QA_EN.into(),
            youtube_video: String::new(),
            audience_mode: "review".into(),
            subtitles: true,
            recording: false,
            actions: Vec::new(),
            action_gap: 12,
            song_requests: false,
            auto_welcome: true,
            ambient_welcome_gap: 45,
            welcome_phrases: default_welcome_phrases_zh(),
            welcome_phrases_en: default_welcome_phrases_en(),
            welcome_joins: true,
            join_welcome_gap: 20,
            auto_thank_gifts: true,
            auto_sing: true,
            auto_sing_gap: 120,
            singing_action: "自然投入地唱歌，神情柔和灵动，随旋律轻轻晃动，情绪高潮处自然闭眼陶醉"
                .into(),
            singing_action_en:
                "Sing naturally with gentle expression, swaying lightly with the melody and getting immersed at emotional parts"
                    .into(),
            post_sing_action: "唱完歌微笑看向镜头，神态温柔放松".into(),
            post_sing_action_en: "Finish singing and smile warmly at the camera, relaxed and natural"
                .into(),
            utterance_gap: 3,
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
    pub fn is_english(&self) -> bool {
        self.live_language == "en"
    }

    pub fn active_tts_model(&self) -> &str {
        if self.is_english() {
            &self.tts_model_en
        } else {
            &self.tts_model
        }
    }

    pub fn active_tts_voice(&self) -> &str {
        if self.is_english() {
            &self.voice_en
        } else {
            &self.voice
        }
    }

    pub fn active_qa_reference(&self) -> &str {
        if self.is_english() {
            &self.qa_reference_en
        } else {
            &self.qa_reference
        }
    }

    pub fn active_welcome_phrases(&self) -> &[String] {
        if self.is_english() {
            &self.welcome_phrases_en
        } else {
            &self.welcome_phrases
        }
    }

    pub fn active_singing_action(&self) -> &str {
        if self.is_english() {
            &self.singing_action_en
        } else {
            &self.singing_action
        }
    }

    pub fn active_post_sing_action(&self) -> &str {
        if self.is_english() {
            &self.post_sing_action_en
        } else {
            &self.post_sing_action
        }
    }

    pub fn uses_local_voice(&self) -> bool {
        self.tts_mode == "mlx" && self.tts_endpoint == LOCAL_TTS
    }

    fn upgrade_local_voice(&mut self) {
        if self.uses_local_voice() {
            self.tts_model = MING.into();
            self.voice = MING_VOICE.into();
            self.tts_model_en = ZONOS.into();
            self.voice_en = ZONOS_VOICE.into();
            self.reference_audio.clear();
            self.reference_text.clear();
            if self.persona == PREVIOUS_PERSONA {
                self.persona = DEFAULT_PERSONA.into();
            }
        }
    }

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
            ["zh", "en"].contains(&self.live_language.as_str()),
            "直播语言仅支持中文（zh）或英文（en）"
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
        ensure!(
            (10..=600).contains(&self.ambient_welcome_gap),
            "定时欢迎间隔须在 10 至 600 秒内"
        );
        ensure!(
            (5..=300).contains(&self.join_welcome_gap),
            "进房欢迎冷却须在 5 至 300 秒内"
        );
        ensure!(
            (15..=3600).contains(&self.auto_sing_gap),
            "定时唱歌间隔须在 15 至 3600 秒内"
        );
        ensure!(
            (1..=60).contains(&self.utterance_gap),
            "行为间隔须在 1 至 60 秒内"
        );
        for phrases in [&self.welcome_phrases, &self.welcome_phrases_en] {
            ensure!(
                phrases.len() <= 50
                    && phrases
                        .iter()
                        .all(|s| !s.trim().is_empty() && s.chars().count() <= 200),
                "定时欢迎话术最多 50 条，每条 1 至 200 字"
            );
        }
        ensure!(
            self.singing_action.chars().count() <= 300
                && self.singing_action_en.chars().count() <= 300
                && self.post_sing_action.chars().count() <= 300
                && self.post_sing_action_en.chars().count() <= 300,
            "唱歌神态指令须在 300 字内"
        );
        if !self.llm_endpoint.is_empty() {
            check_url(&self.llm_endpoint)?;
        }
        if self.tts_mode == "mlx" {
            check_url(&self.tts_endpoint)?;
            ensure!(
                !self.active_tts_model().trim().is_empty(),
                "请填写当前直播语言的语音模型名称"
            );
        }
        if self.uses_local_voice() {
            if self.is_english() {
                ensure!(
                    self.tts_model_en == ZONOS
                        && self.voice_en == ZONOS_VOICE
                        && self.reference_audio.is_empty(),
                    "本机英文语音使用 Willow。自定义模型或参考音色请填写兼容服务地址"
                );
            } else {
                ensure!(
                    self.tts_model == MING
                        && self.voice == MING_VOICE
                        && self.reference_audio.is_empty(),
                    "本机中文语音使用灵小柔。自定义模型或参考音色请填写兼容服务地址"
                );
            }
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
            self.persona.len() <= 16_000
                && self.qa_reference.len() <= 32_000
                && self.qa_reference_en.len() <= 32_000
                && self.reference_text.len() <= 4000,
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
    let mut conf: Settings = if p.exists() {
        read_json(&p)?
    } else {
        Settings::default()
    };
    conf.upgrade_local_voice();
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
fn key_env_vars(name: &str) -> &'static [&'static str] {
    match name {
        "vidu" => &["AVATAR3D_VIDU_KEY", "VIDU_API_KEY", "VIDU_KEY"],
        "llm" => &["AVATAR3D_LLM_KEY", "LLM_API_KEY", "OPENAI_API_KEY"],
        "youtube" => &["AVATAR3D_YOUTUBE_KEY", "YOUTUBE_API_KEY"],
        _ => &[],
    }
}

fn find_project_root() -> Option<PathBuf> {
    if let Ok(cwd) = env::current_dir()
        && (cwd.join(".env").is_file() || cwd.join("Cargo.toml").is_file())
    {
        return Some(cwd);
    }
    if let Ok(exe) = env::current_exe() {
        for dir in exe.ancestors().skip(1) {
            if dir.join(".env").is_file() || dir.join("Cargo.toml").is_file() {
                return Some(dir.to_path_buf());
            }
        }
    }
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if manifest_dir.is_dir() {
        return Some(manifest_dir);
    }
    None
}

pub fn env_file_path() -> Result<PathBuf> {
    if let Some(p) = env::var_os("AVATAR3D_ENV_FILE")
        && !p.is_empty()
    {
        return Ok(PathBuf::from(p));
    }
    if let Some(root) = find_project_root() {
        return Ok(root.join(".env"));
    }
    Ok(data_dir()?.join(".env"))
}

fn parse_env_line(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    let body = trimmed.strip_prefix("export ").map(str::trim_start).unwrap_or(trimmed);
    let (raw_key, raw_val) = body.split_once('=')?;
    let key = raw_key.trim();
    if key.is_empty() {
        return None;
    }
    let val = raw_val.trim();
    let parsed = if val.len() >= 2 && val.starts_with('"') && val.ends_with('"') {
        val[1..val.len() - 1]
            .replace("\\\"", "\"")
            .replace("\\\\", "\\")
    } else if val.len() >= 2 && val.starts_with('\'') && val.ends_with('\'') {
        val[1..val.len() - 1].to_owned()
    } else {
        val.split(" #").next().unwrap_or(val).trim().to_owned()
    };
    Some((key.to_owned(), parsed))
}

fn read_env_value(path: &Path, candidates: &[&str]) -> Result<Option<String>> {
    let text = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("无法读取 {}", path.display())),
    };
    let mut entries = std::collections::HashMap::new();
    for line in text.lines() {
        if let Some((k, v)) = parse_env_line(line) {
            entries.insert(k, v);
        }
    }
    for &c in candidates {
        if let Some(v) = entries.get(c) {
            let trimmed = v.trim();
            if !trimmed.is_empty() {
                return Ok(Some(trimmed.to_owned()));
            }
        }
    }
    Ok(None)
}

fn format_env_value(value: &str) -> String {
    if value
        .chars()
        .any(|c| c.is_whitespace() || matches!(c, '#' | '"' | '\'' | '\\'))
    {
        format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        value.to_owned()
    }
}

fn write_env_key(path: &Path, name: &str, value: &str) -> Result<()> {
    let candidates = key_env_vars(name);
    let canonical = candidates[0];
    let existing = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).with_context(|| format!("无法读取 {}", path.display())),
    };
    let mut lines: Vec<String> = Vec::new();
    let mut updated = false;
    for line in existing.lines() {
        if let Some((k, _)) = parse_env_line(line)
            && candidates.contains(&k.as_str())
        {
            if !value.is_empty() && !updated {
                lines.push(format!("{canonical}={}", format_env_value(value)));
                updated = true;
            }
            continue;
        }
        lines.push(line.to_owned());
    }
    if !value.is_empty() && !updated {
        lines.push(format!("{canonical}={}", format_env_value(value)));
    }
    let mut out = lines.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    atomic(path, out.as_bytes())?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

pub fn key(name: &str) -> Result<Option<String>> {
    ensure!(KEY_NAMES.contains(&name), "未知密钥");
    let candidates = key_env_vars(name);
    for &var in candidates {
        if let Ok(v) = env::var(var)
            && !v.trim().is_empty()
        {
            return Ok(Some(v.trim().to_owned()));
        }
    }
    let path = env_file_path()?;
    read_env_value(&path, candidates)
}

pub fn store_key(name: &str, value: &str) -> Result<()> {
    ensure!(KEY_NAMES.contains(&name), "未知密钥");
    let v = value.trim();
    if !v.is_empty() {
        ensure!(
            v.len() < 8192 && !v.chars().any(char::is_control),
            "密钥格式无效"
        );
    }
    let path = env_file_path()?;
    write_env_key(&path, name, v)
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
        for name in [
            "actions",
            "action_gap",
            "song_requests",
            "live_language",
            "qa_reference",
            "qa_reference_en",
            "auto_welcome",
            "ambient_welcome_gap",
            "welcome_phrases",
            "welcome_phrases_en",
            "welcome_joins",
            "join_welcome_gap",
            "auto_thank_gifts",
            "auto_sing",
            "auto_sing_gap",
            "singing_action",
            "singing_action_en",
            "post_sing_action",
            "post_sing_action_en",
            "utterance_gap",
        ] {
            fields.remove(name);
        }
        let loaded: Settings = serde_json::from_value(old).unwrap();
        loaded.validate().unwrap();
        assert!(loaded.actions.is_empty());
        assert!(!loaded.song_requests);
        assert_eq!(loaded.action_gap, Settings::default().action_gap);
        assert_eq!(loaded.live_language, "zh");
        assert!(loaded.auto_welcome);
        assert!(loaded.welcome_joins);
        assert!(loaded.auto_thank_gifts);
        assert!(loaded.auto_sing);
        assert_eq!(loaded.utterance_gap, 3);
        assert!(loaded.qa_reference.contains("欢迎某某"));
        assert_eq!(loaded.welcome_phrases.len(), 3);
    }

    #[test]
    fn env_file_reads_updates_and_clears_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env");
        fs::write(&path, "# comment\nVIDU_API_KEY=\"vidu-secret\"\nOTHER=1\n").unwrap();

        assert_eq!(
            read_env_value(&path, key_env_vars("vidu")).unwrap().as_deref(),
            Some("vidu-secret")
        );
        assert_eq!(read_env_value(&path, key_env_vars("llm")).unwrap(), None);

        write_env_key(&path, "vidu", "vidu-next").unwrap();
        assert_eq!(
            read_env_value(&path, key_env_vars("vidu")).unwrap().as_deref(),
            Some("vidu-next")
        );
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("# comment"));
        assert!(content.contains("OTHER=1"));
        assert!(content.contains("AVATAR3D_VIDU_KEY=vidu-next"));
        assert!(!content.contains("VIDU_API_KEY"));

        write_env_key(&path, "vidu", "").unwrap();
        assert_eq!(read_env_value(&path, key_env_vars("vidu")).unwrap(), None);
    }
}
