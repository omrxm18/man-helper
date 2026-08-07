use crate::ManError;
use std::io::Write;
use std::process::{Command, Stdio};

/// Pipe raw troff/mdoc source through `mandoc -Thtml`.
///
/// We use mandoc rather than hand-rolling a troff parser: its HTML output
/// is clean and semantic (`<section class="Sh">`, `<dl class="Bl-tag">`,
/// etc.), which is far easier to turn into our `Document` model than
/// re-implementing troff macro expansion from scratch.
pub fn troff_to_html(source: &str) -> Result<String, ManError> {
    let mut child = Command::new("mandoc")
        .arg("-Thtml")
        // fragment: skip <html>/<head> boilerplate, just the body content.
        // man=%N.%S: ask mandoc to emit real <a class="Xr" href="name.section">
        // links for Xr cross-references (e.g. "ls(1)") instead of plain text,
        // so the TUI can let you jump straight to the referenced page.
        .arg("-Ofragment,man=%N.%S")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| ManError::MandocMissing(e.to_string()))?;

    child
        .stdin
        .take()
        .expect("stdin was piped")
        .write_all(source.as_bytes())
        .map_err(ManError::Io)?;

    let output = child.wait_with_output().map_err(ManError::Io)?;
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}