use crossterm::cursor::MoveTo;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::style::{Attribute, Color, Print, ResetColor, SetAttribute, SetForegroundColor};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, size, Clear, ClearType, EnterAlternateScreen,
    LeaveAlternateScreen,
};
use crossterm::{execute, queue};
use manhelper_core::{Block, DefItem, Document, Span as CoreSpan};
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
}

#[derive(Clone)]
struct Word {
    text: String,
    style: Style,
}

/// One logical (unwrapped) line of content: a run of styled words at a
/// given indent depth, plus whether a blank line should follow it once
/// wrapped for display.
struct LogicalLine {
    indent: usize,
    words: Vec<Word>,
    spacer_after: bool,
}

type VisualLine = (usize, Vec<Word>); // (indent, words for this wrapped row)

fn main() -> io::Result<()> {
    let mut args = std::env::args().skip(1);
    let name = match args.next() {
        Some(n) => n,
        None => {
            eprintln!("usage: manview <page> [section]");
            std::process::exit(1);
        }
    };
    let section = args.next();

    // If it looks like a path to a real file, load it directly
    // (handy for testing, or opening a page outside the man database).
    let looks_like_path = name.contains('/') || std::path::Path::new(&name).is_file();
    let doc = if looks_like_path {
        manhelper_core::load_from_path(std::path::Path::new(&name))
    } else {
        manhelper_core::load(&name, section.as_deref())
    };
    let doc = match doc {
        Ok(d) => d,
        Err(e) => {
            eprintln!("manview: {e}");
            std::process::exit(1);
        }
    };

    let title_bar = doc.title.clone();
    let logical = build_lines(&doc);
    run_pager(&logical, &title_bar)
}

fn run_pager(logical: &[LogicalLine], title_bar: &str) -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;

    let mut scroll: usize = 0;
    let (mut cols, mut rows) = size()?;
    let mut visual = wrap_all(logical, cols);

    let result = loop {
        draw(&mut stdout, &visual, scroll, rows, cols, title_bar)?;

        if event::poll(std::time::Duration::from_millis(200))? {
            match event::read()? {
                Event::Resize(new_cols, new_rows) => {
                    cols = new_cols;
                    rows = new_rows;
                    visual = wrap_all(logical, cols);
                    let max_scroll = visual.len().saturating_sub(1);
                    scroll = scroll.min(max_scroll);
                }
                Event::Key(key) => {
                    if key.kind != KeyEventKind::Press {
                        continue;
                    }
                    let page = rows.saturating_sub(2) as usize;
                    let max_scroll = visual.len().saturating_sub(rows.saturating_sub(1) as usize);
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => break Ok(()),
                        KeyCode::Down | KeyCode::Char('j') => {
                            scroll = (scroll + 1).min(max_scroll)
                        }
                        KeyCode::Up | KeyCode::Char('k') => scroll = scroll.saturating_sub(1),
                        KeyCode::PageDown | KeyCode::Char(' ') => {
                            scroll = (scroll + page).min(max_scroll)
                        }
                        KeyCode::PageUp => scroll = scroll.saturating_sub(page),
                        KeyCode::Char('g') | KeyCode::Home => scroll = 0,
                        KeyCode::Char('G') | KeyCode::End => scroll = max_scroll,
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

fn draw(
    stdout: &mut Stdout,
    visual: &[VisualLine],
    scroll: usize,
    rows: u16,
    cols: u16,
    title_bar: &str,
) -> io::Result<()> {
    queue!(stdout, Clear(ClearType::All))?;
    let content_rows = rows.saturating_sub(1);

    for row in 0..content_rows {
        queue!(stdout, MoveTo(0, row))?;
        if let Some((indent, words)) = visual.get(scroll + row as usize) {
            print_line(stdout, *indent, words)?;
        }
    }

    queue!(stdout, MoveTo(0, rows.saturating_sub(1)))?;
    queue!(stdout, SetAttribute(Attribute::Reverse))?;
    let status = format!(" {title_bar}  \u{2014}  q quit \u{b7} j/k scroll \u{b7} g/G top/bottom ");
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

fn print_line(stdout: &mut Stdout, indent: usize, words: &[Word]) -> io::Result<()> {
    queue!(stdout, Print(" ".repeat(indent * 2)))?;
    for (i, w) in words.iter().enumerate() {
        if i > 0 {
            queue!(stdout, Print(" "))?;
        }
        apply_style(stdout, w.style)?;
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
    }
    Ok(())
}

// ---- Document -> logical lines ----

fn build_lines(doc: &Document) -> Vec<LogicalLine> {
    let mut out = Vec::new();

    let mut header_words = vec![Word {
        text: doc.title.clone(),
        style: Style::Title,
    }];
    if !doc.volume.is_empty() {
        header_words.push(Word {
            text: doc.volume.clone(),
            style: Style::Dim,
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
            }],
            spacer_after: false,
        });
        collect_blocks(&section.blocks, 1, &mut out);
        out.push(LogicalLine {
            indent: 0,
            words: vec![],
            spacer_after: false,
        });
    }

    out
}

fn collect_blocks(blocks: &[Block], indent: usize, out: &mut Vec<LogicalLine>) {
    for block in blocks {
        match block {
            Block::Paragraph(spans) => {
                out.push(LogicalLine {
                    indent,
                    words: spans_to_words(spans),
                    spacer_after: true,
                });
            }
            Block::DefList(items) => {
                collect_deflist(items, indent, out);
            }
            Block::Indent(inner) => {
                collect_blocks(inner, indent + 1, out);
            }
            Block::List(items) => {
                for item in items {
                    let mut words = vec![Word {
                        text: "\u{2022}".to_string(),
                        style: Style::Plain,
                    }];
                    words.extend(spans_to_words(item));
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

fn collect_deflist(items: &[DefItem], indent: usize, out: &mut Vec<LogicalLine>) {
    for item in items {
        out.push(LogicalLine {
            indent,
            words: spans_to_words(&item.term)
                .into_iter()
                .map(|w| Word {
                    text: w.text,
                    style: Style::Term,
                })
                .collect(),
            spacer_after: false,
        });
        collect_blocks(&item.body, indent + 1, out);
    }
}

fn spans_to_words(spans: &[CoreSpan]) -> Vec<Word> {
    let mut words = Vec::new();
    for span in spans {
        let (text, style) = match span {
            CoreSpan::Text(t) => (t, Style::Plain),
            CoreSpan::Bold(t) => (t, Style::Bold),
            CoreSpan::Italic(t) => (t, Style::Italic),
            CoreSpan::Code(t) => (t, Style::Code),
        };
        for w in text.split_whitespace() {
            words.push(Word {
                text: w.to_string(),
                style,
            });
        }
    }
    words
}

// ---- word-wrap ----

fn wrap_all(logical: &[LogicalLine], cols: u16) -> Vec<VisualLine> {
    let mut out = Vec::new();
    for line in logical {
        let width = (cols as usize).saturating_sub(line.indent * 2).max(10);
        if line.words.is_empty() {
            out.push((line.indent, Vec::new()));
        } else {
            for wrapped in wrap_words(&line.words, width) {
                out.push((line.indent, wrapped));
            }
        }
        if line.spacer_after {
            out.push((0, Vec::new()));
        }
    }
    out
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
