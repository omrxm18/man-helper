use crossterm::cursor::MoveTo;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::style::{Attribute, Color, Print, ResetColor, SetAttribute, SetForegroundColor};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, size, Clear, ClearType, EnterAlternateScreen,
    LeaveAlternateScreen,
};
use crossterm::{execute, queue};
use core::{Block, DefItem, Document, Span as CoreSpan};
use index::FlagEntry;
use std::collections::HashMap;
use std::io::{self, Stdout, Write};

#[derive(Clone, Copy, PartialEq)]
enum Style {
    Plain,
    Dim,
    Title,
    Heading,
    Term,
    Bold,
    Italic,
    Code,
    Link,
    LinkSelected,
    Match,
    MatchCurrent,
}

#[derive(Clone)]
struct Word {
    text: String,
    style: Style,
    /// Index into the page's `links` table, if this word is (part of) a
    /// followable cross-reference.
    link: Option<usize>,
}

/// One logical (unwrapped) line of content: a run of styled words at a
/// given indent depth, plus whether a blank line should follow it once
/// wrapped for display.
struct LogicalLine {
    indent: usize,
    words: Vec<Word>,
    spacer_after: bool,
}

/// A followable cross-reference target, e.g. "ls(1)" -> name="ls", section="1".
#[derive(Clone)]
struct LinkEntry {
    name: String,
    section: String,
}

type VisualLine = (usize, Vec<Word>); // (indent, words for this wrapped row)

/// Everything needed to display one man page: its parsed+flattened
/// content, its link table, in-page search state, and scroll position.
struct PageState {
    name: String,
    section: Option<String>,
    title_bar: String,
    logical: Vec<LogicalLine>,
    links: Vec<LinkEntry>,
    visual: Vec<VisualLine>,
    link_rows: HashMap<usize, usize>,
    scroll: usize,
    current_link: Option<usize>,

    // In-page search ("/"-triggered, scoped to *this* page only).
    search_active: bool,
    search_query: String,
    /// (row, word-index-within-row) for every match.
    matches: Vec<(usize, usize)>,
    current_match: Option<usize>,
}

impl PageState {
    fn load(name: &str, section: Option<&str>, cols: u16) -> Result<Self, core::ManError> {
        let looks_like_path = name.contains('/') || std::path::Path::new(name).is_file();
        let doc = if looks_like_path {
            core::load_from_path(std::path::Path::new(name))
        } else {
            core::load(name, section)
        }?;

        let title_bar = doc.title.clone();
        let (logical, links) = build_lines(&doc);
        let (visual, link_rows) = wrap_all(&logical, cols);

        Ok(PageState {
            name: name.to_string(),
            section: section.map(String::from),
            title_bar,
            logical,
            links,
            visual,
            link_rows,
            scroll: 0,
            current_link: None,
            search_active: false,
            search_query: String::new(),
            matches: Vec::new(),
            current_match: None,
        })
    }

    fn rewrap(&mut self, cols: u16) {
        let (visual, link_rows) = wrap_all(&self.logical, cols);
        self.visual = visual;
        self.link_rows = link_rows;
        let max_scroll = self.visual.len().saturating_sub(1);
        self.scroll = self.scroll.min(max_scroll);
        if !self.search_query.is_empty() {
            self.recompute_matches(false);
        }
    }

    /// Recompute `matches` for the current `search_query` against this
    /// page's content (case-insensitive substring over each word).
    /// If `jump` is true, move to the nearest match at/after the current
    /// scroll position and scroll it into view.
    fn recompute_matches(&mut self, jump: bool) {
        let query = self.search_query.to_lowercase();
        self.matches.clear();
        if !query.is_empty() {
            for (row, (_, words)) in self.visual.iter().enumerate() {
                for (col, w) in words.iter().enumerate() {
                    if w.text.to_lowercase().contains(&query) {
                        self.matches.push((row, col));
                    }
                }
            }
        }
        if jump {
            self.current_match = self
                .matches
                .iter()
                .position(|&(row, _)| row >= self.scroll)
                .or(if self.matches.is_empty() { None } else { Some(0) });
        } else {
            // Keep pointing at the same match if it still exists, else clear.
            if let Some(idx) = self.current_match {
                if idx >= self.matches.len() {
                    self.current_match = None;
                }
            }
        }
    }
}

fn main() -> io::Result<()> {
    let mut args = std::env::args().skip(1);
    let first = args.next();

    match first.as_deref() {
        Some("flags") => {
            let query: Vec<String> = args.collect();
            flags_mode(&query.join(" "))
        }
        Some("rebuild-index") => {
            rebuild_index_mode();
            Ok(())
        }
        Some(name) => {
            let section = args.next();
            let (cols, _rows) = size()?;
            let page = match PageState::load(name, section.as_deref(), cols) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("manview: {e}");
                    std::process::exit(1);
                }
            };
            run_pager(page)
        }
        None => {
            eprintln!("usage: manview <page> [section]");
            eprintln!("       manview flags <query>      (search flags across every installed page)");
            eprintln!("       manview rebuild-index      (force a fresh flag index)");
            std::process::exit(1);
        }
    }
}

// ---- cross-page flag search: `manview flags <query>` ----
// Folded into the same binary (rather than a separate tool) so it's just
// another mode of `manview`, not a second thing to remember or invoke.

fn flags_mode(query: &str) -> io::Result<()> {
    if query.trim().is_empty() {
        eprintln!("usage: manview flags <query>");
        std::process::exit(1);
    }

    let entries = match index::load_cached() {
        Some(e) => e,
        None => {
            eprintln!("No flag index yet -- building one now (first run only, this can take a moment)...");
            let entries = build_index_with_progress();
            if let Err(e) = index::save_cache(&entries) {
                eprintln!("warning: couldn't save index cache: {e}");
            }
            entries
        }
    };

    let results = index::search(&entries, query);
    if results.is_empty() {
        println!("No matches for \"{query}\".");
        println!("(Tip: run `manview rebuild-index` if you've installed new packages since the index was built.)");
        return Ok(());
    }

    for (i, entry) in results.iter().enumerate() {
        print_flag_entry(i + 1, entry);
    }

    print!("\nOpen which? (number, or Enter to quit): ");
    io::stdout().flush()?;
    let mut input = String::new();
    if io::stdin().read_line(&mut input).unwrap_or(0) == 0 {
        return Ok(()); // EOF / non-interactive invocation
    }
    let choice: usize = match input.trim().parse() {
        Ok(n) if n >= 1 && n <= results.len() => n,
        _ => return Ok(()),
    };

    let entry = results[choice - 1].clone();
    let (cols, _rows) = size()?;
    match PageState::load(&entry.name, Some(&entry.section), cols) {
        Ok(page) => run_pager(page),
        Err(e) => {
            eprintln!("manview: {e}");
            Ok(())
        }
    }
}

fn print_flag_entry(i: usize, entry: &FlagEntry) {
    println!("{i}) {}({})  {}", entry.name, entry.section, entry.flag);
    if !entry.description.is_empty() {
        println!("   {}", entry.description);
    }
}

fn rebuild_index_mode() {
    let entries = build_index_with_progress();
    match index::save_cache(&entries) {
        Ok(()) => println!("Saved index to {}", index::cache_path().display()),
        Err(e) => eprintln!("Failed to save index: {e}"),
    }
}

fn build_index_with_progress() -> Vec<FlagEntry> {
    let pages = index::discover_pages();
    let total = pages.len();
    eprintln!("Found {total} installed man pages.");
    let last_reported = std::sync::atomic::AtomicUsize::new(0);
    let entries = index::build_index(&pages, |done, total| {
        let prev = last_reported.swap(done, std::sync::atomic::Ordering::Relaxed);
        if done == total || done / 50 != prev / 50 {
            eprint!("\rIndexing... {done}/{total}");
            let _ = io::stderr().flush();
        }
    });
    eprintln!();
    eprintln!("Indexed {} flags across {total} pages.", entries.len());
    entries
}

// ---- pager ----

fn run_pager(mut page: PageState) -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;

    // (name, section, scroll-to-restore) for pages we navigated away from.
    let mut history: Vec<(String, Option<String>, usize)> = Vec::new();
    let (mut cols, mut rows) = size()?;

    let result = loop {
        draw(&mut stdout, &page, rows, cols)?;

        if event::poll(std::time::Duration::from_millis(200))? {
            match event::read()? {
                Event::Resize(new_cols, new_rows) => {
                    cols = new_cols;
                    rows = new_rows;
                    page.rewrap(cols);
                }
                Event::Key(key) => {
                    if key.kind != KeyEventKind::Press {
                        continue;
                    }

                    // While typing a search query, keys are captured for that
                    // instead of normal navigation.
                    if page.search_active {
                        match key.code {
                            KeyCode::Esc => {
                                page.search_active = false;
                                page.search_query.clear();
                                page.matches.clear();
                                page.current_match = None;
                            }
                            KeyCode::Enter => {
                                page.search_active = false;
                                page.recompute_matches(true);
                                if let Some(idx) = page.current_match {
                                    let (row, _) = page.matches[idx];
                                    let content_rows = rows.saturating_sub(1) as usize;
                                    scroll_to_row(&mut page, row, content_rows);
                                }
                            }
                            KeyCode::Backspace => {
                                page.search_query.pop();
                                page.recompute_matches(false);
                            }
                            KeyCode::Char(c) => {
                                page.search_query.push(c);
                                page.recompute_matches(false);
                            }
                            _ => {}
                        }
                        continue;
                    }

                    let content_rows = rows.saturating_sub(1) as usize;
                    let page_amount = rows.saturating_sub(2) as usize;
                    let max_scroll = page.visual.len().saturating_sub(content_rows);

                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => break Ok(()),

                        KeyCode::Down | KeyCode::Char('j') => {
                            page.scroll = (page.scroll + 1).min(max_scroll)
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            page.scroll = page.scroll.saturating_sub(1)
                        }
                        KeyCode::PageDown | KeyCode::Char(' ') => {
                            page.scroll = (page.scroll + page_amount).min(max_scroll)
                        }
                        KeyCode::PageUp => page.scroll = page.scroll.saturating_sub(page_amount),
                        KeyCode::Char('g') | KeyCode::Home => page.scroll = 0,
                        KeyCode::Char('G') | KeyCode::End => page.scroll = max_scroll,

                        // In-page search, scoped to this page's own content.
                        KeyCode::Char('/') => {
                            page.search_active = true;
                            page.search_query.clear();
                        }
                        KeyCode::Char('n') if !page.matches.is_empty() => {
                            let next = match page.current_match {
                                Some(i) => (i + 1) % page.matches.len(),
                                None => 0,
                            };
                            page.current_match = Some(next);
                            let (row, _) = page.matches[next];
                            scroll_to_row(&mut page, row, content_rows);
                        }
                        KeyCode::Char('N') if !page.matches.is_empty() => {
                            let prev = match page.current_match {
                                Some(0) | None => page.matches.len() - 1,
                                Some(i) => i - 1,
                            };
                            page.current_match = Some(prev);
                            let (row, _) = page.matches[prev];
                            scroll_to_row(&mut page, row, content_rows);
                        }

                        // Cross-reference navigation.
                        KeyCode::Tab if !page.links.is_empty() => {
                            let next = match page.current_link {
                                Some(i) => (i + 1) % page.links.len(),
                                None => 0,
                            };
                            page.current_link = Some(next);
                            if let Some(&row) = page.link_rows.get(&next) {
                                scroll_to_row(&mut page, row, content_rows);
                            }
                        }
                        KeyCode::BackTab if !page.links.is_empty() => {
                            let prev = match page.current_link {
                                Some(0) | None => page.links.len() - 1,
                                Some(i) => i - 1,
                            };
                            page.current_link = Some(prev);
                            if let Some(&row) = page.link_rows.get(&prev) {
                                scroll_to_row(&mut page, row, content_rows);
                            }
                        }
                        KeyCode::Enter => {
                            if let Some(idx) = page.current_link {
                                let target = page.links[idx].clone();
                                history.push((page.name.clone(), page.section.clone(), page.scroll));
                                match PageState::load(&target.name, Some(&target.section), cols) {
                                    Ok(new_page) => page = new_page,
                                    Err(_) => {
                                        history.pop();
                                    }
                                }
                            }
                        }
                        KeyCode::Backspace | KeyCode::Char('b') => {
                            if let Some((n, s, sc)) = history.pop() {
                                if let Ok(mut restored) = PageState::load(&n, s.as_deref(), cols) {
                                    restored.scroll =
                                        sc.min(restored.visual.len().saturating_sub(content_rows));
                                    page = restored;
                                }
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    };

    disable_raw_mode()?;
    execute!(stdout, LeaveAlternateScreen)?;
    result
}

/// Scroll so the given visual row is in view, centering it when possible.
fn scroll_to_row(page: &mut PageState, row: usize, content_rows: usize) {
    let max_scroll = page.visual.len().saturating_sub(content_rows);
    if row < page.scroll || row >= page.scroll + content_rows {
        page.scroll = row.saturating_sub(content_rows / 2).min(max_scroll);
    }
}

fn draw(stdout: &mut Stdout, page: &PageState, rows: u16, cols: u16) -> io::Result<()> {
    queue!(stdout, Clear(ClearType::All))?;
    let content_rows = rows.saturating_sub(1);

    let current_match_pos = page.current_match.map(|i| page.matches[i]);
    let match_set: std::collections::HashSet<(usize, usize)> = page.matches.iter().copied().collect();

    for row in 0..content_rows {
        let abs_row = page.scroll + row as usize;
        queue!(stdout, MoveTo(0, row))?;
        if let Some((indent, words)) = page.visual.get(abs_row) {
            print_line(
                stdout,
                *indent,
                words,
                abs_row,
                page.current_link,
                &match_set,
                current_match_pos,
            )?;
        }
    }

    queue!(stdout, MoveTo(0, rows.saturating_sub(1)))?;
    queue!(stdout, SetAttribute(Attribute::Reverse))?;
    let status = if page.search_active {
        format!(" /{}", page.search_query)
    } else if !page.search_query.is_empty() {
        let pos = page
            .current_match
            .map(|i| format!("{}/{}", i + 1, page.matches.len()))
            .unwrap_or_else(|| "0/0".to_string());
        format!(
            " {}  \u{2014}  \"{}\" match {pos} \u{b7} n/N next/prev \u{b7} / new search \u{b7} q quit ",
            page.title_bar, page.search_query
        )
    } else {
        let hint = if page.links.is_empty() {
            "q quit \u{b7} j/k scroll \u{b7} g/G top/bottom \u{b7} / search"
        } else {
            "q quit \u{b7} j/k scroll \u{b7} Tab links \u{b7} Enter follow \u{b7} b back \u{b7} / search"
        };
        format!(" {}  \u{2014}  {hint} ", page.title_bar)
    };
    let status = pad_or_trim(&status, cols as usize);
    queue!(stdout, Print(status))?;
    queue!(stdout, SetAttribute(Attribute::Reset))?;
    stdout.flush()
}

fn pad_or_trim(s: &str, width: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() >= width {
        chars[..width].iter().collect()
    } else {
        let mut s = s.to_string();
        s.push_str(&" ".repeat(width - chars.len()));
        s
    }
}

fn print_line(
    stdout: &mut Stdout,
    indent: usize,
    words: &[Word],
    row: usize,
    current_link: Option<usize>,
    match_set: &std::collections::HashSet<(usize, usize)>,
    current_match_pos: Option<(usize, usize)>,
) -> io::Result<()> {
    queue!(stdout, Print(" ".repeat(indent * 2)))?;
    for (i, w) in words.iter().enumerate() {
        if i > 0 {
            queue!(stdout, Print(" "))?;
        }
        let is_current_match = current_match_pos == Some((row, i));
        let is_match = match_set.contains(&(row, i));
        let effective_style = if is_current_match {
            Style::MatchCurrent
        } else if is_match {
            Style::Match
        } else {
            match w.link {
                Some(idx) if Some(idx) == current_link => Style::LinkSelected,
                Some(_) => Style::Link,
                None => w.style,
            }
        };
        apply_style(stdout, effective_style)?;
        queue!(stdout, Print(&w.text))?;
        queue!(stdout, ResetColor, SetAttribute(Attribute::Reset))?;
    }
    Ok(())
}

fn apply_style(stdout: &mut Stdout, style: Style) -> io::Result<()> {
    match style {
        Style::Plain => {}
        Style::Dim => {
            queue!(stdout, SetForegroundColor(Color::DarkGrey))?;
        }
        Style::Title => {
            queue!(
                stdout,
                SetForegroundColor(Color::Yellow),
                SetAttribute(Attribute::Bold)
            )?;
        }
        Style::Heading => {
            queue!(
                stdout,
                SetForegroundColor(Color::Cyan),
                SetAttribute(Attribute::Bold)
            )?;
        }
        Style::Term => {
            queue!(
                stdout,
                SetForegroundColor(Color::Green),
                SetAttribute(Attribute::Bold)
            )?;
        }
        Style::Bold => {
            queue!(stdout, SetAttribute(Attribute::Bold))?;
        }
        Style::Italic => {
            queue!(
                stdout,
                SetForegroundColor(Color::Magenta),
                SetAttribute(Attribute::Italic)
            )?;
        }
        Style::Code => {
            queue!(stdout, SetForegroundColor(Color::Yellow))?;
        }
        Style::Link => {
            queue!(
                stdout,
                SetForegroundColor(Color::Blue),
                SetAttribute(Attribute::Underlined)
            )?;
        }
        Style::LinkSelected => {
            queue!(
                stdout,
                SetForegroundColor(Color::Black),
                crossterm::style::SetBackgroundColor(Color::Blue),
                SetAttribute(Attribute::Bold)
            )?;
        }
        Style::Match => {
            queue!(
                stdout,
                SetForegroundColor(Color::Black),
                crossterm::style::SetBackgroundColor(Color::Yellow)
            )?;
        }
        Style::MatchCurrent => {
            queue!(
                stdout,
                SetForegroundColor(Color::Black),
                crossterm::style::SetBackgroundColor(Color::Green),
                SetAttribute(Attribute::Bold)
            )?;
        }
    }
    Ok(())
}

// ---- Document -> logical lines ----

fn build_lines(doc: &Document) -> (Vec<LogicalLine>, Vec<LinkEntry>) {
    let mut out = Vec::new();
    let mut links: Vec<LinkEntry> = Vec::new();

    let mut header_words = vec![Word {
        text: doc.title.clone(),
        style: Style::Title,
        link: None,
    }];
    if !doc.volume.is_empty() {
        header_words.push(Word {
            text: doc.volume.clone(),
            style: Style::Dim,
            link: None,
        });
    }
    out.push(LogicalLine {
        indent: 0,
        words: header_words,
        spacer_after: true,
    });

    for section in &doc.sections {
        out.push(LogicalLine {
            indent: 0,
            words: vec![Word {
                text: section.heading.clone(),
                style: Style::Heading,
                link: None,
            }],
            spacer_after: false,
        });
        collect_blocks(&section.blocks, 1, &mut out, &mut links);
        out.push(LogicalLine {
            indent: 0,
            words: vec![],
            spacer_after: false,
        });
    }

    (out, links)
}

fn collect_blocks(
    blocks: &[Block],
    indent: usize,
    out: &mut Vec<LogicalLine>,
    links: &mut Vec<LinkEntry>,
) {
    for block in blocks {
        match block {
            Block::Paragraph(spans) => {
                out.push(LogicalLine {
                    indent,
                    words: spans_to_words(spans, links),
                    spacer_after: true,
                });
            }
            Block::DefList(items) => {
                collect_deflist(items, indent, out, links);
            }
            Block::Indent(inner) => {
                collect_blocks(inner, indent + 1, out, links);
            }
            Block::List(items) => {
                for item in items {
                    let mut words = vec![Word {
                        text: "\u{2022}".to_string(),
                        style: Style::Plain,
                        link: None,
                    }];
                    words.extend(spans_to_words(item, links));
                    out.push(LogicalLine {
                        indent,
                        words,
                        spacer_after: false,
                    });
                }
                if let Some(last) = out.last_mut() {
                    last.spacer_after = true;
                }
            }
        }
    }
}

fn collect_deflist(
    items: &[DefItem],
    indent: usize,
    out: &mut Vec<LogicalLine>,
    links: &mut Vec<LinkEntry>,
) {
    for item in items {
        let mut term_words = spans_to_words(&item.term, links);
        for w in &mut term_words {
            if w.link.is_none() {
                w.style = Style::Term;
            }
        }
        out.push(LogicalLine {
            indent,
            words: term_words,
            spacer_after: false,
        });
        collect_blocks(&item.body, indent + 1, out, links);
    }
}

fn spans_to_words(spans: &[CoreSpan], links: &mut Vec<LinkEntry>) -> Vec<Word> {
    let mut words = Vec::new();
    for span in spans {
        match span {
            CoreSpan::Text(t) => push_words(&mut words, t, Style::Plain, None),
            CoreSpan::Bold(t) => push_words(&mut words, t, Style::Bold, None),
            CoreSpan::Italic(t) => push_words(&mut words, t, Style::Italic, None),
            CoreSpan::Code(t) => push_words(&mut words, t, Style::Code, None),
            CoreSpan::Link { text, name, section } => {
                links.push(LinkEntry {
                    name: name.clone(),
                    section: section.clone(),
                });
                let idx = links.len() - 1;
                push_words(&mut words, text, Style::Link, Some(idx));
            }
        }
    }
    words
}

fn push_words(words: &mut Vec<Word>, text: &str, style: Style, link: Option<usize>) {
    for w in text.split_whitespace() {
        words.push(Word {
            text: w.to_string(),
            style,
            link,
        });
    }
}

// ---- word-wrap ----

/// Wrap logical lines to the given width, and record the first visual
/// row each link appears on (so Tab can scroll straight to it).
fn wrap_all(logical: &[LogicalLine], cols: u16) -> (Vec<VisualLine>, HashMap<usize, usize>) {
    let mut out: Vec<VisualLine> = Vec::new();
    let mut link_rows: HashMap<usize, usize> = HashMap::new();

    for line in logical {
        let width = (cols as usize).saturating_sub(line.indent * 2).max(10);
        if line.words.is_empty() {
            out.push((line.indent, Vec::new()));
        } else {
            for wrapped in wrap_words(&line.words, width) {
                let row = out.len();
                for w in &wrapped {
                    if let Some(idx) = w.link {
                        link_rows.entry(idx).or_insert(row);
                    }
                }
                out.push((line.indent, wrapped));
            }
        }
        if line.spacer_after {
            out.push((0, Vec::new()));
        }
    }
    (out, link_rows)
}

fn wrap_words(words: &[Word], width: usize) -> Vec<Vec<Word>> {
    let mut lines = Vec::new();
    let mut cur: Vec<Word> = Vec::new();
    let mut cur_len = 0usize;

    for w in words {
        let wlen = w.text.chars().count();
        let add = if cur.is_empty() { wlen } else { wlen + 1 };
        if cur_len + add > width && !cur.is_empty() {
            lines.push(std::mem::take(&mut cur));
            cur_len = 0;
        }
        if !cur.is_empty() {
            cur_len += 1;
        }
        cur.push(w.clone());
        cur_len += wlen;
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    if lines.is_empty() {
        lines.push(Vec::new());
    }
    lines
}