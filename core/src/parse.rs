use crate::document::{Block, DefItem, Document, Section, Span};
use regex::Regex;
use scraper::{ElementRef, Html, Node, Selector};
use std::sync::OnceLock;

pub fn parse_document(html: &str) -> Document {
    let doc = Html::parse_fragment(html);

    let title = select_text(&doc, "td.head-ltitle");
    let volume = select_text(&doc, "td.head-vol");

    let section_sel = Selector::parse("div.manual-text > section.Sh").unwrap();
    let h_sel = Selector::parse(":scope > h1").unwrap();

    let mut sections = Vec::new();
    for sec in doc.select(&section_sel) {
        let (heading, id) = match sec.select(&h_sel).next() {
            Some(h) => (
                collapse(&h.text().collect::<String>()),
                h.value().attr("id").unwrap_or_default().to_string(),
            ),
            None => (String::new(), String::new()),
        };
        let blocks = parse_blocks(sec);
        sections.push(Section {
            id,
            heading,
            blocks,
        });
    }

    Document {
        title,
        volume,
        sections,
    }
}

fn select_text(doc: &Html, selector: &str) -> String {
    let sel = Selector::parse(selector).unwrap();
    doc.select(&sel)
        .next()
        .map(|e| collapse(&e.text().collect::<String>()))
        .unwrap_or_default()
}

/// Walk the direct children of an element and turn them into Blocks.
/// Unrecognized wrapper elements (nested <section> for Ss subsections,
/// stray <div>s) are flattened rather than dropped, so content never
/// silently disappears even if we don't have a dedicated Block variant.
fn parse_blocks(el: ElementRef) -> Vec<Block> {
    let mut blocks = Vec::new();

    for child in el.children() {
        let Some(child_el) = ElementRef::wrap(child) else {
            continue;
        };
        let name = child_el.value().name();
        let class = child_el.value().attr("class").unwrap_or("");

        match name {
            "p" => {
                let spans = spans_from_node(child);
                if !spans.is_empty() {
                    blocks.push(Block::Paragraph(spans));
                }
            }
            "dl" => {
                blocks.push(Block::DefList(parse_deflist(child_el)));
            }
            "ul" | "ol" => {
                let items: Vec<Vec<Span>> = child_el
                    .children()
                    .filter_map(ElementRef::wrap)
                    .filter(|e| e.value().name() == "li")
                    .map(|li| spans_from_node(*li))
                    .collect();
                blocks.push(Block::List(items));
            }
            "div" if class.contains("Bd-indent") || class.contains("Bd") => {
                blocks.push(Block::Indent(parse_blocks(child_el)));
            }
            "section" => {
                // Nested Ss subsection: keep its sub-heading as a bold
                // paragraph, then flatten its content into an Indent block.
                let mut inner = Vec::new();
                if let Some(h) = child_el
                    .children()
                    .filter_map(ElementRef::wrap)
                    .find(|e| e.value().name().starts_with('h'))
                {
                    inner.push(Block::Paragraph(vec![Span::Bold(collapse(
                        &h.text().collect::<String>(),
                    ))]));
                }
                inner.extend(parse_blocks(child_el));
                blocks.push(Block::Indent(inner));
            }
            "table" => {
                // Skip head/foot chrome tables; real content tables are rare
                // in man pages and can be added as a Block::Table later.
            }
            _ => {
                // Unknown wrapper: recurse so nested <p>/<dl>/etc still surface.
                blocks.extend(parse_blocks(child_el));
            }
        }
    }

    blocks
}

fn parse_deflist(dl: ElementRef) -> Vec<DefItem> {
    let mut items = Vec::new();
    let mut pending_term: Option<Vec<Span>> = None;

    for child in dl.children() {
        let Some(el) = ElementRef::wrap(child) else {
            continue;
        };
        match el.value().name() {
            "dt" => {
                // flush an orphaned term with an empty body (rare, but be safe)
                if let Some(term) = pending_term.take() {
                    items.push(DefItem {
                        term,
                        body: Vec::new(),
                    });
                }
                pending_term = Some(spans_from_node(child));
            }
            "dd" => {
                let body = parse_blocks(el);
                let body = if body.is_empty() {
                    let spans = spans_from_node(child);
                    if spans.is_empty() {
                        Vec::new()
                    } else {
                        vec![Block::Paragraph(spans)]
                    }
                } else {
                    body
                };
                items.push(DefItem {
                    term: pending_term.take().unwrap_or_default(),
                    body,
                });
            }
            _ => {}
        }
    }
    if let Some(term) = pending_term.take() {
        items.push(DefItem {
            term,
            body: Vec::new(),
        });
    }
    items
}

/// Collect all descendant text/bold/italic/code content of a node into
/// a flat list of inline spans, preserving basic emphasis and turning
/// cross-references into followable Links.
fn spans_from_node(node: ego_tree::NodeRef<Node>) -> Vec<Span> {
    let mut raw = Vec::new();
    collect_spans(node, &mut raw);
    // Plain-text fallback: most Linux man pages are written with classic
    // man(7) macros, not mdoc's semantic .Xr, so "ls(1)" in SEE ALSO often
    // arrives as bare text rather than a real <a class="Xr">. Detect that
    // pattern in any leftover Text spans and turn it into a Link too.
    let split: Vec<Span> = raw
        .into_iter()
        .flat_map(|s| match s {
            Span::Text(t) => split_xrefs(&t),
            other => vec![other],
        })
        .collect();
    // A second, extremely common GNU pattern: the command name is bolded
    // and the "(1)" section marker is a separate adjacent plain-text node
    // (e.g. `<b>dircolors</b>(1)`), so the pass above never sees them as
    // one string. Merge that pattern into a Link here.
    merge_bold_xrefs(split)
}

fn merge_bold_xrefs(spans: Vec<Span>) -> Vec<Span> {
    let mut out = Vec::with_capacity(spans.len());
    let mut iter = spans.into_iter().peekable();
    while let Some(span) = iter.next() {
        let name = match &span {
            Span::Bold(t) | Span::Code(t) if is_xref_name(t) => Some(t.clone()),
            _ => None,
        };
        if let Some(name) = name {
            if let Some(Span::Text(next)) = iter.peek() {
                if let Some((section, rest)) = strip_leading_section(next) {
                    iter.next(); // consume the "(N)" text node
                    out.push(Span::Link {
                        text: format!("{name}({section})"),
                        name,
                        section,
                    });
                    if !rest.is_empty() {
                        out.extend(split_xrefs(&rest));
                    }
                    continue;
                }
            }
        }
        out.push(span);
    }
    out
}

/// Is this bold/code text plausibly a command/page name (as opposed to,
/// say, a flag like "-a" or an arbitrary bolded word)?
fn is_xref_name(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// If `text` starts with (optionally, whitespace then) "(<section>)",
/// return the section and whatever remains after it.
fn strip_leading_section(text: &str) -> Option<(String, String)> {
    let trimmed = text.trim_start_matches(' ');
    let after_open = trimmed.strip_prefix('(')?;
    let close = after_open.find(')')?;
    let section = &after_open[..close];
    let mut sec_chars = section.chars();
    let starts_digit = matches!(sec_chars.next(), Some(c) if c.is_ascii_digit());
    if !starts_digit || !sec_chars.all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    Some((section.to_string(), after_open[close + 1..].to_string()))
}

fn xref_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"([A-Za-z][A-Za-z0-9_.:+-]*)\((\d[A-Za-z]{0,3})\)").unwrap())
}

fn split_xrefs(text: &str) -> Vec<Span> {
    let re = xref_regex();
    let mut out = Vec::new();
    let mut last = 0;
    for caps in re.captures_iter(text) {
        let m = caps.get(0).unwrap();
        if m.start() > last {
            out.push(Span::Text(text[last..m.start()].to_string()));
        }
        out.push(Span::Link {
            text: m.as_str().to_string(),
            name: caps[1].to_string(),
            section: caps[2].to_string(),
        });
        last = m.end();
    }
    if out.is_empty() {
        return vec![Span::Text(text.to_string())];
    }
    if last < text.len() {
        out.push(Span::Text(text[last..].to_string()));
    }
    out
}

fn collect_spans(node: ego_tree::NodeRef<Node>, out: &mut Vec<Span>) {
    for child in node.children() {
        match child.value() {
            Node::Text(text) => {
                let s = normalize_ws(&text.text);
                if !s.is_empty() {
                    out.push(Span::Text(s));
                }
            }
            Node::Element(elem) => {
                let tag = elem.name();
                let class = elem.attr("class").unwrap_or("");
                match tag {
                    "a" if class.contains("Xr") => {
                        if let Some((name, section)) = elem
                            .attr("href")
                            .and_then(|href| href.rsplit_once('.'))
                            .map(|(n, s)| (n.to_string(), s.to_string()))
                        {
                            let text = collapse(&text_of(child));
                            out.push(Span::Link {
                                text,
                                name,
                                section,
                            });
                        } else {
                            collect_spans(child, out);
                        }
                    }
                    "b" | "strong" => {
                        let t = collapse(&text_of(child));
                        if !t.is_empty() {
                            out.push(Span::Bold(t));
                        }
                    }
                    "i" | "em" => {
                        let t = collapse(&text_of(child));
                        if !t.is_empty() {
                            out.push(Span::Italic(t));
                        }
                    }
                    "code" => {
                        let t = collapse(&text_of(child));
                        if !t.is_empty() {
                            out.push(Span::Code(t));
                        }
                    }
                    // links, spans, and other inline wrappers: recurse through
                    _ => collect_spans(child, out),
                }
            }
            _ => {}
        }
    }
}

fn text_of(node: ego_tree::NodeRef<Node>) -> String {
    let mut s = String::new();
    for descendant in node.descendants() {
        if let Node::Text(t) = descendant.value() {
            s.push_str(&t.text);
        }
    }
    s
}

/// Collapse all internal whitespace to single spaces and trim ends.
fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Like `collapse`, but preserves a single leading/trailing space when the
/// original text had one, since that space is often meaningful between
/// adjacent inline spans (e.g. "See " before a bold cross-reference).
fn normalize_ws(s: &str) -> String {
    if s.trim().is_empty() {
        return if s.is_empty() { String::new() } else { " ".to_string() };
    }
    let leading = s.starts_with(|c: char| c.is_whitespace());
    let trailing = s.ends_with(|c: char| c.is_whitespace());
    let mut out = collapse(s);
    if leading {
        out.insert(0, ' ');
    }
    if trailing {
        out.push(' ');
    }
    out
}