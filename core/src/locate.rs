use crate::ManError;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Ask `man -w` where a page lives on disk without invoking a pager.
/// `section` is optional, e.g. Some("5") for man(5) pages.
pub fn find_man_page(name: &str, section: Option<&str>) -> Result<PathBuf, ManError> {
    let mut cmd = Command::new("man");
    cmd.arg("-w");
    if let Some(s) = section {
        cmd.arg(s);
    }
    cmd.arg(name);

    let output = cmd.output().map_err(ManError::Io)?;
    if !output.status.success() {
        return Err(ManError::NotFound(name.to_string()));
    }

    let path_str = String::from_utf8_lossy(&output.stdout);
    let first_line = path_str
        .lines()
        .next()
        .ok_or_else(|| ManError::NotFound(name.to_string()))?;
    Ok(PathBuf::from(first_line.trim()))
}

/// Read a man page file, transparently gunzipping if needed, and return
/// the raw troff/mdoc source.
pub fn read_raw(path: &Path) -> Result<String, ManError> {
    let bytes = std::fs::read(path).map_err(ManError::Io)?;

    let is_gz = path.extension().and_then(|e| e.to_str()) == Some("gz");
    if is_gz {
        let mut decoder = flate2::read::GzDecoder::new(&bytes[..]);
        let mut out = String::new();
        decoder
            .read_to_string(&mut out)
            .map_err(ManError::Io)?;
        Ok(out)
    } else {
        String::from_utf8(bytes).map_err(|e| ManError::Utf8(e.to_string()))
    }
}
