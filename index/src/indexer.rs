use crate::finder::PageRef;
use manrender_core::{Block, DefItem, Document, Span};
use serde::{Deserialize, Serialize};
use std::sync::mpsc;
use std::thread;

/// One flag/option found in some man page's definition list, e.g.
/// "-a, --all" in ls(1) becomes two FlagEntry rows: "-a" and "--all",
/// both pointing at ls/1 with the same description.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlagEntry {
    pub flag: String,
    pub name: String,
    pub section: String,
    pub description: String,
}

/// Parse every given page (in parallel across a small thread pool) and
/// collect every flag/option it documents. `on_progress` is called once
/// per page processed (successful or not) so callers can show a counter.
pub fn build_index(pages: &[PageRef], on_progress: impl Fn(usize, usize) + Send + Sync) -> Vec<FlagEntry> {
    let total = pages.len();
    let worker_count = thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(total.max(1));

    let (tx, rx) = mpsc::channel::<Vec<FlagEntry>>();
    let done = std::sync::atomic::AtomicUsize::new(0);

    thread::scope(|scope| {
        for chunk in chunk_evenly(pages, worker_count) {
            let tx = tx.clone();
            let done = &done;
            let on_progress = &on_progress;
            scope.spawn(move || {
                let mut local = Vec::new();
                for page in chunk {
                    if let Ok(doc) = manrender_core::load_from_path(&page.path) {
                        collect_flags(&doc, &page.name, &page.section, &mut local);
                    }
                    let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                    on_progress(n, total);
                }
                let _ = tx.send(local);
            });
        }
        drop(tx);
    });

    let mut all = Vec::new();
    for chunk in rx {
        all.extend(chunk);
    }
    all
}

fn chunk_evenly<T: Clone>(items: &[T], n: usize) -> Vec<Vec<T>> {
    if n == 0 || items.is_empty() {
        return vec![items.to_vec()];
    }
    let chunk_size = items.len().div_ceil(n);
    items.chunks(chunk_size.max(1)).map(|c| c.to_vec()).collect()
}

fn collect_flags(doc: &Document, name: &str, section: &str, out: &mut Vec<FlagEntry>) {
    for sec in &doc.sections {
        collect_flags_blocks(&sec.blocks, name, section, out);
    }
}

fn collect_flags_blocks(blocks: &[Block], name: &str, section: &str, out: &mut Vec<FlagEntry>) {
    for block in blocks {
        match block {
            Block::DefList(items) => collect_defitems(items, name, section, out),
            Block::Indent(inner) => collect_flags_blocks(inner, name, section, out),
            _ => {}
        }
    }
}

fn collect_defitems(items: &[DefItem], name: &str, section: &str, out: &mut Vec<FlagEntry>) {
    for item in items {
        let term_text = plain_text(&item.term);
        let description = first_paragraph_text(&item.body);

        // A term is often "-a, --all" or "-x, --exclude=PATTERN" — index
        // each comma-separated flag on its own so both "-a" and "--all"
        // are independently searchable.
        for raw in term_text.split(',') {
            let flag = raw.trim();
            if flag.is_empty() {
                continue;
            }
            out.push(FlagEntry {
                flag: flag.to_string(),
                name: name.to_string(),
                section: section.to_string(),
                description: description.clone(),
            });
        }

        // Recurse into the body in case of nested DefLists (rare, but
        // some pages nest option groups).
        collect_flags_blocks(&item.body, name, section, out);
    }
}

fn plain_text(spans: &[Span]) -> String {
    spans.iter().map(|s| s.plain_text()).collect::<Vec<_>>().join("")
}

fn first_paragraph_text(blocks: &[Block]) -> String {
    for block in blocks {
        if let Block::Paragraph(spans) = block {
            let text = plain_text(spans);
            if !text.is_empty() {
                return truncate(&text, 100);
            }
        }
    }
    String::new()
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let truncated: String = s.chars().take(max_chars).collect();
    format!("{truncated}\u{2026}")
}