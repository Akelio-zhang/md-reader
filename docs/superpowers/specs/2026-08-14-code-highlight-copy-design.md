# Code Block Syntax Highlighting and Copy Button

Date: 2026-08-14
Status: Approved (design phase)

## Background

md-reader renders Markdown locally with a GitHub-like preview style. Code
blocks currently render as plain `<pre><code>` with no syntax highlighting,
and there is no way to copy a code block from the preview. The tool's core
constraints are: fully offline, single self-contained HTML output (preview
mode), and light/dark themes following `prefers-color-scheme`.

## Goal

1. Syntax-highlight fenced code blocks using the **lumis** crate
   (Tree-sitter-based, Neovim-derived themes), with light and dark theme
   variants that follow the reader's existing `prefers-color-scheme` scheme.
2. Add a copy button to every code block, available in both preview and
   serve modes.

## Non-goals

- Client-side highlighting (embedded highlight.js) — rejected: ~1 MB added
  to every output file, contradicts the lightweight offline positioning.
- Theme switching UI — the reader follows the system color scheme, as today.
- Highlighting errors surfaced to the reader — silently fall back to
  plain text (unlike KaTeX errors, which are visible).

## Architecture

All changes live in the existing `render` module (`src/render.rs`), plus
CSS in `assets/reader.css` and one new inline script.

### Data flow

```
pulldown-cmark events
  └─ Event::CodeBlock(lang, text)
       ├─ lang recognized by lumis → Event::Html(lumis multi-theme output)
       └─ no lang / unknown / failure → unchanged (plain <pre><code>)
```

`render_markdown` already maps math events (`InlineMath`/`DisplayMath`) to
`Event::Html`; code blocks follow the same pattern.

### Dependencies

```toml
[dependencies]
lumis = { version = "0.5", default-features = false, features = [
  "lang-rust", "lang-python", "lang-javascript", "lang-typescript",
  "lang-json", "lang-html", "lang-css", "lang-bash", "lang-sql",
  "lang-markdown", "lang-go", "lang-java", "lang-c", "lang-cpp", "lang-yaml",
] }
```

Exact feature flag names to be verified against the crate; if a language's
flag is missing or renamed, adjust the list (target stays ~15 common
languages). `default-features = false` keeps compile time and binary size
bounded.

### Highlighting (server-side)

- A single `highlight_code(lang: &str, code: &str) -> Option<String>`
  helper builds one lumis `HtmlMultiThemesBuilder` (cached in a `OnceLock`),
  with themes named `light` and `dark` (GitHub light/dark or nearest
  equivalent), and returns `None` for unknown languages or formatting
  errors → caller falls back to the plain code block.
- The builder's output shape (wrapper elements, theme class names) will be
  inspected at implementation time; the CSS must key off the emitted
  light/dark markers, matching the reader's existing `prefers-color-scheme`
  switching.

### Copy button

- One inline `<script>` (shared by preview and serve modes, ~20 lines):
  on pointer enter (or keyboard focus) a "Copy" button appears at the
  top-right of each `pre`; clicking it writes `pre.innerText` via
  `navigator.clipboard` with a fallback to `execCommand('copy')` for
  non-secure contexts; button label briefly switches to "Copied".
- CSS in `assets/reader.css`: button position, visibility on
  `:hover`/`:focus-within`, copied state. Reuses existing `--border`,
  `--surface`, `--accent` variables so it follows light/dark automatically.
- Preview mode gains this script unconditionally (it is tiny); serve mode
  keeps its existing live-reload script alongside.

### Error handling

- Unknown language → no highlight (current behavior preserved).
- lumis formatting error → no highlight (silent fallback).
- Clipboard API unavailable → button does nothing visible except the
  execCommand fallback path; no error surfaced.

## Testing (TDD)

- Highlighted fenced block with a recognized language emits lumis output
  containing both light and dark theme markers.
- Code block without a language → plain `<pre><code>`, unchanged.
- Unknown language (e.g. `lang="notalanguage"`) → plain, unchanged.
- Both preview and serve HTML include the copy script; the copy script is
  present exactly once per document.
- `reader_css()` contains the copy button styles.
- Existing tests (math, delimiters, routes, TCP) stay green.

## Cost

- First compile with the 15-language subset: an estimated 1-3 minutes;
  incremental builds unaffected. Release binary grows by roughly 2-3 MB.
- Acceptable for a local tool; noted in the commit message.
