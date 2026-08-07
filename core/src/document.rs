//! The helper-agnostic representation of a parsed man page.
//! TUI, HTML, and GUI renderers all consume this same structure.

#[derive(Debug, Clone, Default)]
pub struct Document {
    /// e.g. "JAVA(1)"
    pub title: String,
    /// e.g. "JDK Commands"
    pub volume: String,
    pub sections: Vec<Section>,
}

#[derive(Debug, Clone, Default)]
pub struct Section {
    /// Anchor id, e.g. "DESCRIPTION"
    pub id: String,
    pub heading: String,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone)]
pub enum Block {
    Paragraph(Vec<Span>),
    /// Definition list: flags/terms and their descriptions (Bl-tag in mdoc)
    DefList(Vec<DefItem>),
    /// Indented block (Bd-indent), holds nested blocks
    Indent(Vec<Block>),
    /// Bullet or numbered list items
    List(Vec<Vec<Span>>),
}

#[derive(Debug, Clone)]
pub struct DefItem {
    pub term: Vec<Span>,
    pub body: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Span {
    Text(String),
    Bold(String),
    Italic(String),
    Code(String),
    /// A cross-reference to another man page, e.g. "ls(1)".
    /// `name`/`section` are what gets passed to `load()` to follow it.
    Link {
        text: String,
        name: String,
        section: String,
    },
}

impl Span {
    pub fn plain_text(&self) -> &str {
        match self {
            Span::Text(s) | Span::Bold(s) | Span::Italic(s) | Span::Code(s) => s,
            Span::Link { text, .. } => text,
        }
    }
}