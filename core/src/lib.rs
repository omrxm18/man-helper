pub mod document;
mod locate;
mod parse;
mod troff;

pub use document::{Block, DefItem, Document, Section, Span};
pub use locate::find_man_page;
pub use troff::troff_to_html;

use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ManError {
    #[error("man page not found: {0}")]
    NotFound(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid utf8 in man page source: {0}")]
    Utf8(String),
    #[error("couldn't run mandoc (is it installed?): {0}")]
    MandocMissing(String),
}

/// Locate, decompress, and parse a man page by name (e.g. "ls") and
/// optional section (e.g. Some("5")), returning the structured Document
/// that all renderers (TUI/HTML/GUI) build on.
pub fn load(name: &str, section: Option<&str>) -> Result<Document, ManError> {
    let path = find_man_page(name, section)?;
    load_from_path(&path)
}

/// Parse a man page directly from a file on disk (gzipped or plain),
/// bypassing `man -w` lookup. Useful for testing or opening a page that
/// isn't installed in the system man database.
pub fn load_from_path(path: &Path) -> Result<Document, ManError> {
    let raw = locate::read_raw(path)?;
    let html = troff::troff_to_html(&raw)?;
    Ok(parse::parse_document(&html))
}
