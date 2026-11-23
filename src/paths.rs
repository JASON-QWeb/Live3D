use crate::config::data_dir;
use anyhow::Result;
use std::{
    env, fs, io,
    path::{Path, PathBuf},
    sync::OnceLock,
};

const KNOWLEDGE: [&str; 2] = [
    "avatars/cn/xiaorou_knowledge_base.txt",
    "avatars/en/knowledge_base.txt",
];
const SONG_TITLES: [&str; 2] = ["avatars/cn/song_titles.txt", "avatars/en/song_titles.txt"];

pub fn bundle_contents() -> Option<PathBuf> {
    let exe = env::current_exe().ok()?;
    let macos = exe.parent()?;
    if macos.ends_with("Contents/MacOS") {
        macos.parent().map(Path::to_owned)
    } else {
        None
    }
}

fn project_root() -> Option<&'static Path> {
    static ROOT: OnceLock<Option<PathBuf>> = OnceLock::new();
    ROOT.get_or_init(|| {
        if let Some(contents) = bundle_contents() {
            // Only a bundle still inside this checkout uses its live project files.
            return contents
                .ancestors()
                .skip(2)
                .find(|p| {
                    p.join("Cargo.toml").is_file() && p.join("packaging/Info.plist").is_file()
                })
                .map(Path::to_owned);
        }
        let is_project = |p: &Path| p.join(".env").is_file() || p.join("Cargo.toml").is_file();
        if let Ok(cwd) = env::current_dir()
            && is_project(&cwd)
        {
            return Some(cwd);
        }
        if let Ok(exe) = env::current_exe()
            && let Some(root) = exe.ancestors().skip(1).find(|p| is_project(p))
        {
            return Some(root.to_owned());
        }
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        root.is_dir().then(|| root.to_owned())
    })
    .as_deref()
}

fn resource_root() -> Result<PathBuf> {
    project_root().map(Path::to_owned).map_or_else(data_dir, Ok)
}

pub fn qa_file_path(english: bool) -> Option<PathBuf> {
    Some(resource_root().ok()?.join(KNOWLEDGE[usize::from(english)]))
}

pub fn song_titles_file_path(english: bool) -> Option<PathBuf> {
    Some(
        resource_root()
            .ok()?
            .join(SONG_TITLES[usize::from(english)]),
    )
}

pub fn env_file_path() -> Result<PathBuf> {
    if let Some(p) = env::var_os("AVATAR3D_ENV_FILE")
        && !p.is_empty()
    {
        return Ok(p.into());
    }
    Ok(resource_root()?.join(".env"))
}

pub fn prepare_bundled_avatars(data: &Path) -> Result<()> {
    let Some(contents) = bundle_contents() else {
        return Ok(());
    };
    if project_root().is_some() {
        return Ok(());
    }
    for relative in KNOWLEDGE.into_iter().chain(SONG_TITLES) {
        let source = contents.join("Resources").join(relative);
        let dest = data.join(relative);
        if !source.is_file() || dest.exists() {
            continue;
        }
        let parent = dest.parent().expect("avatar path has a parent");
        fs::create_dir_all(parent)?;
        let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
        io::copy(&mut fs::File::open(source)?, &mut tmp)?;
        if let Err(err) = tmp.persist_noclobber(dest)
            && err.error.kind() != io::ErrorKind::AlreadyExists
        {
            return Err(err.error.into());
        }
    }
    Ok(())
}
