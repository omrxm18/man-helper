use std::path::{Path, PathBuf};
use std::process::Command;

/// One discovered man page on disk, with its name/section already
/// parsed out of the filename (e.g. "ls.1.gz" -> name="ls", section="1").
#[derive(Debug, Clone)]
pub struct PageRef {
    pub path: PathBuf,
    pub name: String,
    pub section: String,
}

/// Find every man page installed on the system, across all `manpath`
/// directories (falls back to the usual default locations if the
/// `manpath` tool isn't available for some reason).
pub fn discover_pages() -> Vec<PageRef> {
    let mut out = Vec::new();
    for dir in manpath_dirs() {
        walk(&dir, &mut out);
    }
    out
}

fn manpath_dirs() -> Vec<PathBuf> {
    let output = Command::new("manpath").output();
    match output {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout);
            let dirs: Vec<PathBuf> = text.trim().split(':').map(PathBuf::from).collect();
            if !dirs.is_empty() {
                return dirs;
            }
        }
        _ => {}
    }
    vec![
        PathBuf::from("/usr/share/man"),
        PathBuf::from("/usr/local/share/man"),
    ]
}

fn walk(dir: &Path, out: &mut Vec<PageRef>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            // Skip preformatted-cache directories ("cat1", "cat3", ...):
            // same content as the real man*/ dirs, would just duplicate
            // every entry in the index.
            let dirname = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if dirname.starts_with("cat") {
                continue;
            }
            walk(&path, out);
        } else if let Some(page) = parse_page_filename(&path) {
            out.push(page);
        }
    }
}

fn parse_page_filename(path: &Path) -> Option<PageRef> {
    let filename = path.file_name()?.to_str()?;
    let stem = filename.strip_suffix(".gz").unwrap_or(filename);
    let (name, section) = stem.rsplit_once('.')?;
    if name.is_empty() || section.is_empty() {
        return None;
    }
    // Section is normally a digit (optionally followed by letters, e.g.
    // "3p", "1x") or "n". Filters out things like locale ".mo" files
    // that might share the directory tree.
    let mut chars = section.chars();
    let first = chars.next()?;
    if !(first.is_ascii_digit() || section == "n") {
        return None;
    }
    Some(PageRef {
        path: path.to_path_buf(),
        name: name.to_string(),
        section: section.to_string(),
    })
}