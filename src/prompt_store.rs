use crate::{
    config,
    prompts::{Knowledge, Profile, Scene, Snapshot},
    state::{App, lock},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, hash_map::DefaultHasher},
    fs,
    hash::{Hash, Hasher},
    io::{ErrorKind, Read, Write},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime},
};

#[derive(Default, Deserialize)]
pub struct LegacyPrompts {
    persona: Option<String>,
    qa_reference: Option<String>,
    qa_reference_en: Option<String>,
    welcome_phrases: Option<Vec<String>>,
    welcome_phrases_en: Option<Vec<String>>,
}

impl LegacyPrompts {
    pub fn take(settings: &mut serde_json::Value) -> Result<Self> {
        let settings = settings.as_object_mut().context("设置须为 JSON 对象")?;
        let fields = [
            "persona",
            "qa_reference",
            "qa_reference_en",
            "welcome_phrases",
            "welcome_phrases_en",
        ];
        let legacy = fields
            .into_iter()
            .filter_map(|field| settings.remove(field).map(|value| (field.into(), value)))
            .collect();
        Ok(serde_json::from_value(serde_json::Value::Object(legacy))?)
    }

    fn initial(&self, english: bool, avatar_file: Option<&Path>) -> Profile {
        let mut profile = Profile::defaults(english);
        let default_inline = match &profile.knowledge {
            Knowledge::Inline { text } => Some(text.clone()),
            Knowledge::AvatarFile => None,
        };
        if let Some(persona) = &self.persona {
            profile.persona.clone_from(persona);
        }
        let (qa, welcome) = if english {
            (&self.qa_reference_en, &self.welcome_phrases_en)
        } else {
            (&self.qa_reference, &self.welcome_phrases)
        };
        let avatar_text = avatar_file.and_then(|path| read_text(path, 32_000).ok());
        let custom_qa = qa.as_deref().map(str::trim).filter(|text| {
            !text.is_empty()
                && default_inline.as_deref().map(str::trim) != Some(*text)
                && avatar_text.as_deref().map(str::trim) != Some(*text)
        });
        if custom_qa.is_some()
            && let Some(text) = qa
        {
            profile.knowledge = Knowledge::Inline { text: text.clone() };
        } else if avatar_file.is_some() {
            profile.knowledge = Knowledge::AvatarFile;
        } else if let Some(text) = qa {
            profile.knowledge = Knowledge::Inline { text: text.clone() };
        }
        if let Some(welcome) = welcome {
            profile.events.insert(Scene::Ambient, welcome.clone());
        }
        let source = match &profile.knowledge {
            Knowledge::Inline { text } => Some(text.clone()),
            Knowledge::AvatarFile => avatar_text,
        };
        if let Some(source) = source {
            profile.retain_resolved_defaults(&source);
        }
        profile
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Files {
    profile: Option<(SystemTime, u64, u64)>,
    knowledge: Option<(SystemTime, u64, u64)>,
}

#[derive(Default)]
struct Entry {
    files: Option<Files>,
    active: Option<Arc<Snapshot>>,
    error: Option<String>,
}

#[derive(Serialize)]
pub struct Document {
    path: PathBuf,
    knowledge_path: Option<PathBuf>,
    revision: String,
    active_version: Option<String>,
    error: Option<String>,
    text: String,
}

#[derive(Deserialize)]
pub struct Edit {
    revision: String,
    text: String,
}

pub struct Store {
    root: PathBuf,
    entries: Mutex<HashMap<(String, bool), Entry>>,
}

impl Store {
    pub fn new(dir: &Path, id: &str, legacy: &LegacyPrompts) -> Result<Self> {
        let store = Self {
            root: dir.join("prompts"),
            entries: Mutex::new(HashMap::new()),
        };
        store.prepare(id, Some(legacy))?;
        Ok(store)
    }

    pub fn prepare(&self, id: &str, legacy: Option<&LegacyPrompts>) -> Result<()> {
        fs::create_dir_all(self.root.join(id))?;
        for english in [false, true] {
            let path = self.path(id, english);
            if !path.exists() {
                let profile = match legacy {
                    Some(legacy) => legacy.initial(english, avatar_path(english).as_deref()),
                    None => LegacyPrompts::default().initial(english, None),
                };
                let mut tmp = tempfile::NamedTempFile::new_in(self.root.join(id))?;
                tmp.write_all(serde_json::to_vec_pretty(&profile)?.as_slice())?;
                tmp.as_file().sync_all()?;
                if let Err(error) = tmp.persist_noclobber(&path)
                    && error.error.kind() != ErrorKind::AlreadyExists
                {
                    return Err(error.error.into());
                }
            }
            self.refresh(id, english);
        }
        Ok(())
    }

    pub fn active(&self, id: &str, english: bool) -> Result<Arc<Snapshot>> {
        let entries = lock(&self.entries);
        let entry = entries
            .get(&(id.into(), english))
            .context("话术档案尚未加载")?;
        entry.active.clone().with_context(|| {
            format!(
                "话术档案未生效：{}",
                entry.error.as_deref().unwrap_or("尚未加载")
            )
        })
    }

    pub fn document(&self, id: &str, english: bool) -> Result<Document> {
        self.refresh(id, english);
        let path = self.path(id, english);
        let text = read_document(&path)?;
        let knowledge_path = serde_json::from_str::<Profile>(&text)
            .ok()
            .and_then(|profile| {
                matches!(profile.knowledge, Knowledge::AvatarFile)
                    .then(|| config::qa_file_path(english))
                    .flatten()
            });
        let entries = lock(&self.entries);
        let entry = entries
            .get(&(id.into(), english))
            .context("话术档案尚未加载")?;
        Ok(Document {
            path,
            knowledge_path,
            revision: fingerprint(&text),
            active_version: entry.active.as_ref().map(|active| active.version.clone()),
            error: entry.error.clone(),
            text,
        })
    }

    pub fn save(&self, id: &str, english: bool, edit: Edit) -> Result<Document> {
        let path = self.path(id, english);
        let avatar = config::qa_file_path(english);
        ensure!(edit.text.len() <= 256_000, "话术档案过大");
        {
            let mut entries = lock(&self.entries);
            let current = read_document(&path)?;
            ensure!(
                fingerprint(&current) == edit.revision,
                "话术档案已在磁盘上修改，请重新加载后保存"
            );
            compile(&edit.text, english, avatar.as_deref())?;
            config::atomic(&path, edit.text.as_bytes())?;
            entries.entry((id.into(), english)).or_default().files = None;
        }
        self.document(id, english)
    }

    fn path(&self, id: &str, english: bool) -> PathBuf {
        self.root
            .join(id)
            .join(if english { "en.json" } else { "zh.json" })
    }

    fn refresh(&self, id: &str, english: bool) {
        let path = self.path(id, english);
        let avatar = config::qa_file_path(english);
        let mut entries = lock(&self.entries);
        let entry = entries.entry((id.into(), english)).or_default();
        let stamp = files(&path, avatar.as_deref());
        if entry.files == Some(stamp) {
            return;
        }
        entry.files = Some(stamp);
        match read_document(&path).and_then(|text| compile(&text, english, avatar.as_deref())) {
            Ok(snapshot) => {
                entry.active = Some(Arc::new(snapshot));
                entry.error = None;
            }
            Err(error) => entry.error = Some(error.to_string()),
        }
    }
}

pub async fn watch(app: App) {
    let mut tick = tokio::time::interval(Duration::from_millis(500));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = app.quit.cancelled() => break,
            _ = tick.tick() => {
                let id = lock(&app.conf).prompt_profile.clone();
                app.prompts.refresh(&id, false);
                app.prompts.refresh(&id, true);
            }
        }
    }
}

fn avatar_path(english: bool) -> Option<PathBuf> {
    config::qa_file_path(english).filter(|path| path.is_file())
}

fn fingerprint(text: &str) -> String {
    let mut state = DefaultHasher::new();
    text.hash(&mut state);
    format!("{:016x}", state.finish())
}

fn files(profile: &Path, avatar: Option<&Path>) -> Files {
    fn stamp(path: &Path) -> Option<(SystemTime, u64, u64)> {
        let meta = fs::metadata(path).ok()?;
        Some((meta.modified().ok()?, meta.len(), meta.ino()))
    }
    Files {
        profile: stamp(profile),
        knowledge: avatar.and_then(stamp),
    }
}

fn read_document(path: &Path) -> Result<String> {
    match read_text(path, 256_000) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error).with_context(|| format!("无法读取 {}", path.display())),
    }
}

fn read_text(path: &Path, limit: u64) -> std::io::Result<String> {
    let mut text = String::new();
    fs::File::open(path)?
        .take(limit + 1)
        .read_to_string(&mut text)?;
    if text.len() as u64 > limit {
        return Err(std::io::Error::new(ErrorKind::InvalidData, "话术文件过大"));
    }
    Ok(text)
}

fn compile(text: &str, english: bool, avatar: Option<&Path>) -> Result<Snapshot> {
    let profile: Profile = serde_json::from_str(text).context("话术档案 JSON 格式无效")?;
    let reference = match &profile.knowledge {
        Knowledge::Inline { text } => text.clone(),
        Knowledge::AvatarFile => {
            let path = avatar.context("找不到角色提示词路径")?;
            let reference =
                read_text(path, 32_000).with_context(|| format!("无法读取 {}", path.display()))?;
            ensure!(
                !reference.trim().is_empty(),
                "角色提示词文件为空：{}",
                path.display()
            );
            reference
        }
    };
    let version = format!("{}-{}", fingerprint(text), fingerprint(&reference));
    profile.compile(&reference, english, version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_edits_do_not_overwrite_disk_and_invalid_reload_keeps_valid_profile() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store {
            root: dir.path().into(),
            entries: Mutex::default(),
        };
        store.prepare("xiaorou", None).unwrap();
        let first = store.document("xiaorou", false).unwrap();
        let edited = first.text.clone() + "\n";
        config::atomic(&first.path, edited.as_bytes()).unwrap();
        let stale = Edit {
            revision: first.revision,
            text: first.text,
        };
        let result = store.save("xiaorou", false, stale);
        assert!(matches!(result, Err(error) if error.to_string().contains("已在磁盘上修改")));
        let current = store.document("xiaorou", false).unwrap();
        assert_eq!(current.text, edited);
        assert_ne!(current.active_version, first.active_version);

        config::atomic(&first.path, b"{").unwrap();
        let invalid = store.document("xiaorou", false).unwrap();
        assert!(invalid.error.is_some());
        assert_eq!(invalid.active_version, current.active_version);
    }

    #[test]
    fn legacy_migration_preserves_custom_qa_over_avatar_file() {
        let dir = tempfile::tempdir().unwrap();
        let avatar = dir.path().join("xiaorou_knowledge_base.txt");
        fs::write(&avatar, "Q：默认问题\nA：默认回答\n").unwrap();

        let custom = LegacyPrompts {
            qa_reference: Some("Q：我的自定义问题\nA：我的自定义回答".into()),
            ..Default::default()
        };
        let migrated_custom = custom.initial(false, Some(&avatar));
        assert!(matches!(
            migrated_custom.knowledge,
            Knowledge::Inline { ref text } if text.contains("我的自定义回答")
        ));

        let untouched = LegacyPrompts {
            qa_reference: Some("Q：默认问题\nA：默认回答".into()),
            ..Default::default()
        };
        let migrated_default = untouched.initial(false, Some(&avatar));
        assert!(matches!(migrated_default.knowledge, Knowledge::AvatarFile));
    }
}
