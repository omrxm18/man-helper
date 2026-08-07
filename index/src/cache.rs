use crate::indexer::FlagEntry;
use std::path::PathBuf;

pub fn cache_path() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."));
            home.join(".cache")
        });
    base.join("manrender").join("flags_index.json")
}

pub fn save(entries: &[FlagEntry]) -> std::io::Result<()> {
    let path = cache_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = std::fs::File::create(&path)?;
    serde_json::to_writer(file, entries).map_err(std::io::Error::other)
}

pub fn load() -> Option<Vec<FlagEntry>> {
    let path = cache_path();
    let file = std::fs::File::open(path).ok()?;
    serde_json::from_reader(file).ok()
}