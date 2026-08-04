# manview

Renders man pages beautifully — plus real features `man`/`less`/bat don't
have: clickable cross-page navigation, in-page search, and cross-page
flag search. All one binary: `manview`.

## Crates

- **`manview-core`** — the shared engine. Locates a page (`man -w`),
  decompresses it, pipes the raw troff/mdoc through
  `mandoc -Ofragment,man=%N.%S -Thtml` (asks mandoc to emit real
  `<a class="Xr" href="name.section">` links for cross-references), then
  parses that into a renderer-agnostic `Document` model (sections →
  blocks → styled spans, including a `Span::Link` variant). Since most
  Linux man pages use classic man(7) macros rather than mdoc's semantic
  `.Xr`, the parser also detects "ls(1)"-shaped plain text, and the very
  common GNU pattern of a bold command name immediately followed by a
  separate `(1)` text node (`<b>dircolors</b>(1)`), turning both into
  followable links.

- **`manview-index`** — a library (not its own binary) that walks
  every man page across all `manpath` directories, parses each with
  `manview-core`, and extracts every flag/option from its definition
  lists (e.g. ls(1)'s "-a, --all" becomes two independently-searchable
  entries). Runs in parallel across your CPU cores, and caches the
  result as JSON at `$XDG_CACHE_HOME/manview/flags_index.json` (or
  `~/.cache/manview/...`). Matching is **token-aware**, not raw
  substring: flags are split on their natural delimiters (`-`, `:`, `=`,
  `_`, `.`), and the query must match a whole token. This matters —  a
  naive substring search for "all" would otherwise match
  `-XX:AllocateHeapAt=path` (since "all" is a fragment of "Allocate"),
  which is technically true but useless noise. Token matching correctly
  finds `--all` and `--almost-all` while excluding `AllocateHeapAt`.

- **`manview-tui`** (`manview`) — the one binary, with two modes:

  **Viewing a page** — `manview ls`, `manview printf 3`, or `manview
  ./some.1` directly. A terminal pager built on `crossterm` directly (no
  `ratatui` — its dependency chain needed newer rustc than this sandbox
  had; feel free to switch if yours doesn't have that constraint).
  Colors headings/terms/bold/italic/code/links.
  - `Tab` / `Shift+Tab` — cycle cross-reference links on the page
  - `Enter` — follow the selected link into that man page
  - `b` / `Backspace` — go back (full history stack, like a browser)
  - `/` — search *within this page*: type live, `Enter` jumps to the
    nearest match and highlights every match (current match in green,
    others in yellow), `n`/`N` cycle through them, `Esc` cancels
  - `j`/`k`/arrows/`PageUp`/`PageDown`/`g`/`G` — scroll, live-resizes

  **Cross-page flag search** — `manview flags dry-run` searches every
  installed page's flags at once (this is the thing `apropos` can't do,
  since apropos only searches page *descriptions*, never the flags
  documented inside them). First run builds the index automatically;
  after that it's instant. `manview rebuild-index` forces a fresh one
  (e.g. after installing new packages — the cache doesn't auto-invalidate
  yet). Picking a result opens it in the *same process* — no spawning a
  second binary.

## Build

```
cargo build --release
```

Needs `mandoc` installed. Note: on Arch, `mandoc` conflicts with man-db's
`man`/`apropos`/`whatis`/`mman` binaries (both packages ship the same
paths) — either extract just the `mandoc` binary from the package cache
without fully installing it, or switch your system to mandoc as the
`man` provider entirely (mandoc's own `man -w` works fine either way).
it can still work with the `man-db` package but compatability isn't guaranteed.