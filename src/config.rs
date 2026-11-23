pub use crate::paths::{env_file_path, qa_file_path, song_titles_file_path};
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
pub const KEY_NAMES: [&str; 5] = ["vidu", "llm", "youtube", "obs", "douyin"];
const LOCAL_TTS: &str = "http://127.0.0.1:19849/v1/audio/speech";
pub const MING: &str = "jorgepb96/Ming-3.0-tts-0.5B-4bit";
pub const ZONOS: &str = "mlx-community/ZONOS3-Mini";
pub const MING_VOICE: &str = "灵小柔";
pub const ZONOS_VOICE: &str = "willow";
fn strip_audio_ext(s: &str) -> &str {
    let trimmed = s.trim().trim_start_matches('\u{feff}').trim();
    const EXTS: [&str; 9] = [
        ".mp3", ".wav", ".flac", ".m4a", ".aac", ".ogg", ".opus", ".mp4", ".mov",
    ];
    let lower = trimmed.to_ascii_lowercase();
    for ext in EXTS {
        if lower.ends_with(ext) && trimmed.len() > ext.len() {
            return trimmed[..trimmed.len() - ext.len()].trim();
        }
    }
    trimmed
}

fn strip_cover_suffix(s: &str) -> &str {
    let mut cur = s.trim();
    const SUFFIXES: [&str; 7] = [
        "－小柔演唱",
        "-小柔演唱",
        "—小柔演唱",
        "_小柔演唱",
        "（小柔演唱）",
        "(小柔演唱)",
        "小柔演唱",
    ];
    loop {
        let mut changed = false;
        for suf in SUFFIXES {
            if let Some(rest) = cur.strip_suffix(suf) {
                let next = rest
                    .trim_end_matches(|c: char| {
                        c.is_whitespace() || matches!(c, '-' | '－' | '—' | '–' | '_' | '·')
                    })
                    .trim();
                if !next.is_empty() {
                    cur = next;
                    changed = true;
                    break;
                }
            }
        }
        if !changed {
            break;
        }
    }
    cur
}

fn normalize_song_key(raw: &str) -> String {
    let base = strip_cover_suffix(strip_audio_ext(raw));
    let mut out = String::with_capacity(base.len());
    let mut prev_space = false;
    for ch in base.chars() {
        if ch.is_whitespace() {
            if !prev_space && !out.is_empty() {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.push(ch.to_ascii_lowercase());
            prev_space = false;
        }
    }
    out.trim().to_owned()
}

fn split_song_mapping_line(line: &str) -> Option<(&str, &str)> {
    let trimmed = line.trim().trim_start_matches('\u{feff}').trim();
    if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//") {
        return None;
    }
    for sep in ["=>", "->", "=", "＝", "\t"] {
        if let Some((k, v)) = trimmed.split_once(sep) {
            let key = k.trim();
            let val = v.trim();
            if !key.is_empty() && !val.is_empty() {
                return Some((key, val));
            }
        }
    }
    None
}

fn strip_leading_track_number(s: &str) -> &str {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == 0 || i == bytes.len() {
        return s;
    }
    let rest = &s[i..];
    if let Some(after_delim) = rest.strip_prefix(['.', '、', '_', '-', '－', ' ', '\t']) {
        let cleaned = after_delim.trim_start_matches(['.', '、', '_', '-', '－', ' ', '\t']);
        if !cleaned.is_empty() {
            return cleaned;
        }
    }
    // Handle compact prefixes like "97云烟成雨" or "101红豆" while preserving titles like "2002年的第一场雪".
    if let Some(first) = rest.chars().next()
        && !first.is_ascii()
        && !matches!(
            first,
            '年' | '月' | '日' | '度' | '分' | '秒' | '号' | '岁' | '夜' | '天'
        )
    {
        return rest.trim();
    }
    s
}

fn strip_bracketed_notes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth_paren = 0u32;
    let mut depth_bracket = 0u32;
    for ch in s.chars() {
        match ch {
            '(' | '（' => depth_paren += 1,
            ')' | '）' if depth_paren > 0 => depth_paren -= 1,
            '[' | '【' => depth_bracket += 1,
            ']' | '】' if depth_bracket > 0 => depth_bracket -= 1,
            _ if depth_paren == 0 && depth_bracket == 0 => out.push(ch),
            _ => {}
        }
    }
    out.trim().to_owned()
}

pub fn clean_fallback_song_title(raw: &str) -> String {
    let stem = strip_audio_ext(raw);
    let base = strip_cover_suffix(stem);
    let no_num = strip_leading_track_number(base);
    let first_part = no_num
        .split(['-', '－', '—', '–'])
        .map(str::trim)
        .find(|part| !part.is_empty())
        .unwrap_or(no_num);
    let cleaned = strip_bracketed_notes(first_part);
    if cleaned.is_empty() {
        stem.to_owned()
    } else {
        cleaned
    }
}

pub fn resolve_song_title_from_text(raw: &str, mapping_text: Option<&str>) -> String {
    let stem = strip_audio_ext(raw);
    if stem.is_empty() {
        return raw.trim().to_owned();
    }
    if let Some(text) = mapping_text {
        let query_norm = normalize_song_key(stem);
        let mut normalized_match: Option<&str> = None;
        for line in text.lines() {
            if let Some((k, v)) = split_song_mapping_line(line) {
                if strip_audio_ext(k) == stem {
                    return v.to_owned();
                }
                if normalized_match.is_none() && normalize_song_key(k) == query_norm {
                    normalized_match = Some(v);
                }
            }
        }
        if let Some(v) = normalized_match {
            return v.to_owned();
        }
    }
    clean_fallback_song_title(stem)
}

pub fn resolve_song_title(cfg: &Settings, raw: &str) -> String {
    let mapping = song_titles_file_path(cfg.is_english())
        .and_then(|p| fs::read_to_string(p).ok())
        .filter(|s| s.len() <= 256_000);
    resolve_song_title_from_text(raw, mapping.as_deref())
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
    pub prompt_profile: String,
    pub youtube_video: String,
    pub audience_mode: String,
    pub subtitles: bool,
    pub recording: bool,
    pub actions: Vec<String>,
    pub action_gap: u32,
    pub song_requests: bool,
    pub auto_welcome: bool,
    pub ambient_welcome_gap: u32,
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
    pub song_audio_mode: String,
    pub song_sync_offset_ms: u32,
    pub song_volume: u32,
    pub bgm_volume: u32,
    pub bgm_duck_percent: u32,
    pub obs_port: u16,
    /// Where comments come from: "douyin", "youtube" or "other" (external tools
    /// posting to /api/ingest). Empty until the operator picks one.
    pub live_platform: String,
    pub douyin_room: String,
    pub douyin_port: u16,
    pub douyin_program: String,
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
            prompt_profile: "xiaorou".into(),
            youtube_video: String::new(),
            audience_mode: "review".into(),
            subtitles: false,
            recording: false,
            actions: Vec::new(),
            action_gap: 12,
            song_requests: false,
            auto_welcome: true,
            ambient_welcome_gap: 45,
            welcome_joins: true,
            join_welcome_gap: 20,
            auto_thank_gifts: true,
            auto_sing: true,
            auto_sing_gap: 120,
            singing_action: "自然投入地唱歌，神情柔和灵动，随旋律轻轻晃动".into(),
            singing_action_en:
                "Sing naturally with gentle, expressive demeanor, swaying lightly with the melody"
                    .into(),
            post_sing_action: "唱完歌微笑看向镜头，神态温柔放松".into(),
            post_sing_action_en:
                "Finish singing and smile warmly at the camera, relaxed and natural".into(),
            utterance_gap: 3,
            song_audio_mode: "stage".into(),
            song_sync_offset_ms: 800,
            song_volume: 95,
            bgm_volume: 25,
            bgm_duck_percent: 30,
            obs_port: 4455,
            live_platform: String::new(),
            douyin_room: String::new(),
            douyin_port: 1089,
            douyin_program: String::new(),
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

    /// Settings saved before the platform choice existed keep the source they had set up.
    fn upgrade_platform(&mut self) {
        if !self.live_platform.is_empty() {
            return;
        }
        if !self.douyin_room.trim().is_empty() {
            self.live_platform = "douyin".into();
        } else if !self.youtube_video.is_empty() {
            self.live_platform = "youtube".into();
        }
    }

    fn upgrade_local_voice(&mut self) {
        if self.uses_local_voice() {
            self.tts_model = MING.into();
            self.voice = MING_VOICE.into();
            self.tts_model_en = ZONOS.into();
            self.voice_en = ZONOS_VOICE.into();
            self.reference_audio.clear();
            self.reference_text.clear();
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
        ensure!(self.obs_port > 0, "OBS WebSocket 端口无效");
        ensure!(
            ["zh", "en"].contains(&self.live_language.as_str()),
            "直播语言仅支持中文（zh）或英文（en）"
        );
        ensure!(
            (1..=64).contains(&self.prompt_profile.len())
                && self
                    .prompt_profile
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b)),
            "话术档案名称仅支持 1 至 64 个英文字母、数字、连字符或下划线"
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
        ensure!(
            ["stage", "vidu"].contains(&self.song_audio_mode.as_str()),
            "演唱出声方式仅支持 stage 或 vidu"
        );
        ensure!(
            self.song_sync_offset_ms <= 4000,
            "音画同步偏移须在 0 至 4000 毫秒内"
        );
        ensure!(
            self.song_volume <= 100 && self.bgm_volume <= 100 && self.bgm_duck_percent <= 100,
            "音量与闪避比例须在 0 至 100 之间"
        );
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
            ["", "douyin", "youtube", "other"].contains(&self.live_platform.as_str()),
            "直播平台无效"
        );
        ensure!(
            self.douyin_room.trim().is_empty()
                || crate::douyin::room_id(&self.douyin_room).is_some(),
            "抖音直播间号无效，请填直播间号或 live.douyin.com 链接"
        );
        ensure!(self.douyin_port > 0, "douyinLive 端口无效");
        let program = self.douyin_program.trim();
        ensure!(
            program.is_empty()
                || (program.starts_with('/')
                    && program.len() <= 1024
                    && !program.chars().any(char::is_control)),
            "douyinLive 程序路径须为完整路径，以 / 开头"
        );
        ensure!(self.reference_text.len() <= 4000, "文本过长");
        ensure!(
            self.reference_audio.is_empty() == self.reference_text.trim().is_empty(),
            "参考录音和逐字稿须一同填写"
        );
        Ok(())
    }
}
pub fn data_dir() -> Result<PathBuf> {
    if let Some(p) = env::var_os("VORELA_DATA_DIR").or_else(|| env::var_os("AVATAR3D_DATA_DIR")) {
        return Ok(p.into());
    }
    let support = PathBuf::from(env::var_os("HOME").context("无法定位用户目录")?)
        .join("Library/Application Support");
    let current = support.join("Vorela");
    let legacy = support.join("Avatar3D Live");
    // Existing settings and model environments can contain absolute paths.
    if !current.exists() && legacy.is_dir() {
        return Ok(legacy);
    }
    Ok(current)
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
pub fn load_settings(dir: &Path) -> Result<(Settings, crate::prompt_store::LegacyPrompts)> {
    let p = dir.join("settings.json");
    let (mut conf, legacy): (Settings, _) = if p.exists() {
        let mut value = read_json(&p)?;
        let legacy = crate::prompt_store::LegacyPrompts::take(&mut value)?;
        (
            serde_json::from_value(value).with_context(|| format!("无法读取 {}", p.display()))?,
            legacy,
        )
    } else {
        (Settings::default(), Default::default())
    };
    conf.upgrade_local_voice();
    conf.upgrade_platform();
    conf.validate()?;
    Ok((conf, legacy))
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
        "vidu" => &[
            "VORELA_VIDU_KEY",
            "AVATAR3D_VIDU_KEY",
            "VIDU_API_KEY",
            "VIDU_KEY",
        ],
        "llm" => &[
            "VORELA_LLM_KEY",
            "AVATAR3D_LLM_KEY",
            "LLM_API_KEY",
            "OPENAI_API_KEY",
        ],
        "youtube" => &[
            "VORELA_YOUTUBE_KEY",
            "AVATAR3D_YOUTUBE_KEY",
            "YOUTUBE_API_KEY",
        ],
        "obs" => &["VORELA_OBS_PASSWORD", "AVATAR3D_OBS_PASSWORD"],
        "douyin" => &["VORELA_DOUYIN_COOKIE", "AVATAR3D_DOUYIN_COOKIE"],
        _ => &[],
    }
}

fn parse_env_line(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    let body = trimmed
        .strip_prefix("export ")
        .map(str::trim_start)
        .unwrap_or(trimmed);
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
            "prompt_profile",
            "auto_welcome",
            "ambient_welcome_gap",
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
            "song_audio_mode",
            "song_sync_offset_ms",
            "song_volume",
            "bgm_volume",
            "bgm_duck_percent",
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
        assert_eq!(loaded.song_audio_mode, "stage");
        assert_eq!(loaded.song_sync_offset_ms, 800);
        assert_eq!(loaded.song_volume, 95);
        assert_eq!(loaded.bgm_volume, 25);
        assert_eq!(loaded.bgm_duck_percent, 30);
        assert_eq!(loaded.prompt_profile, "xiaorou");
        assert!(!loaded.singing_action.contains("闭眼"));
        assert!(!loaded.singing_action.contains("陶醉"));
    }

    #[test]
    fn platform_choice_carries_over_from_older_settings() {
        let dir = tempfile::tempdir().unwrap();
        let save = |patch: serde_json::Value| {
            let mut v = serde_json::to_value(Settings::default()).unwrap();
            let fields = v.as_object_mut().unwrap();
            fields.remove("live_platform");
            fields.extend(patch.as_object().unwrap().clone());
            write_json(&dir.path().join("settings.json"), &v).unwrap();
            load_settings(dir.path()).unwrap().0.live_platform
        };
        assert_eq!(save(serde_json::json!({})), "");
        assert_eq!(
            save(serde_json::json!({"youtube_video":"dQw4w9WgXcQ"})),
            "youtube"
        );
        assert_eq!(
            save(serde_json::json!({"douyin_room":"516466932480"})),
            "douyin"
        );
        assert_eq!(
            save(serde_json::json!({"live_platform":"other","youtube_video":"dQw4w9WgXcQ"})),
            "other"
        );
        let unknown = Settings {
            live_platform: "bilibili".into(),
            ..Settings::default()
        };
        assert!(unknown.validate().is_err());
    }

    #[test]
    fn env_file_reads_updates_and_clears_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env");
        fs::write(&path, "# comment\nVIDU_API_KEY=\"vidu-secret\"\nOTHER=1\n").unwrap();

        assert_eq!(
            read_env_value(&path, key_env_vars("vidu"))
                .unwrap()
                .as_deref(),
            Some("vidu-secret")
        );
        assert_eq!(read_env_value(&path, key_env_vars("llm")).unwrap(), None);

        write_env_key(&path, "vidu", "vidu-next").unwrap();
        assert_eq!(
            read_env_value(&path, key_env_vars("vidu"))
                .unwrap()
                .as_deref(),
            Some("vidu-next")
        );
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("# comment"));
        assert!(content.contains("OTHER=1"));
        assert!(content.contains("VORELA_VIDU_KEY=vidu-next"));
        assert!(!content.contains("VIDU_API_KEY"));

        write_env_key(&path, "vidu", "").unwrap();
        assert_eq!(read_env_value(&path, key_env_vars("vidu")).unwrap(), None);
    }

    #[test]
    fn resolves_song_titles_from_mapping_and_fallback() {
        let map = "\
# comment
223. 海伦-桥边姑娘－小柔演唱 = 桥边姑娘
97云烟成雨-《我是江小白》动画片尾曲 - 房东的猫－小柔演唱 = 云烟成雨
安和桥-宋冬野 = 安和桥
";
        assert_eq!(
            resolve_song_title_from_text("223. 海伦-桥边姑娘－小柔演唱", Some(map)),
            "桥边姑娘"
        );
        // Works with or without the "－小柔演唱" suffix and file extension
        assert_eq!(
            resolve_song_title_from_text("安和桥-宋冬野－小柔演唱.mp3", Some(map)),
            "安和桥"
        );
        assert_eq!(
            resolve_song_title_from_text("223. 海伦-桥边姑娘", Some(map)),
            "桥边姑娘"
        );
        // Full-width equals sign from Chinese IME also works
        assert_eq!(
            resolve_song_title_from_text(
                "陈奕迅-好久不见－小柔演唱",
                Some("陈奕迅-好久不见 ＝ 好久不见")
            ),
            "好久不见"
        );
        // Fallback cleans up unmapped filenames cleanly while keeping numeric song titles
        assert_eq!(
            resolve_song_title_from_text(
                "97云烟成雨-《我是江小白》动画片尾曲 - 房东的猫－小柔演唱",
                None
            ),
            "云烟成雨"
        );
        assert_eq!(
            resolve_song_title_from_text("090.旅行的意义(国) - 陈绮贞－小柔演唱", None),
            "旅行的意义"
        );
        assert_eq!(
            resolve_song_title_from_text("2002年的第一场雪 - 刀郎", None),
            "2002年的第一场雪"
        );
    }
}
