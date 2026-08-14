# Code Highlighting and Copy Button Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Syntax-highlight fenced code blocks with lumis (GitHub light/dark themes following `prefers-color-scheme`) and add a copy button to every code block in both preview and serve modes.

**Architecture:** `render_markdown` in `src/render.rs` already maps pulldown-cmark events to `Event::Html` for math; `Event::CodeBlock(lang, text)` gets the same treatment via a new `highlight_code` helper that returns `None` (fall back to plain rendering) for unknown languages or formatting errors. A small always-on inline script adds copy buttons; CSS lives in `assets/reader.css`.

**Tech Stack:** Rust, pulldown-cmark (existing), lumis 0.13 (Tree-sitter-based highlighting, `HtmlMultiThemesBuilder`), no new JS/CSS dependencies.

## Global Constraints

- Fully offline: highlighting must happen server-side at render time; no CDN, no client-side highlighter.
- Preview mode output must remain a single self-contained HTML file.
- Light/dark switching follows `prefers-color-scheme` (reader's existing scheme).
- lumis features (already in Cargo.toml, verified by `cargo add`): `default-features = false` + `lang-rust, lang-python, lang-javascript, lang-typescript, lang-json, lang-html, lang-css, lang-bash, lang-sql, lang-markdown, lang-go, lang-java, lang-c, lang-cpp, lang-yaml`.
- Unknown language / empty language / highlight failure → plain `<pre><code>`, identical to current output. No error surfaced to the reader.
- Themes: `themes::get("github_light")` and `themes::get("github_dark")` (both verified to exist; `themes::get` returns `Result`).
- Copy button uses `navigator.clipboard` with an `execCommand('copy')` fallback (required for `file://` previews, where the Clipboard API is unavailable).
- Tests first (TDD), per existing suite style in `src/render.rs` tests module.

---
### Task 1: Syntax highlighting with lumis

**Files:**
- Modify: `src/render.rs` (imports, `highlight_code`, `render_markdown` CodeBlock arm, theme CSS)
- Modify: `assets/reader.css` (lumis dark-theme overrides)
- Test: `src/render.rs` tests module

**Interfaces:**
- Consumes: lumis API — `Language::guess(Option<&str>, &str) -> Language`, `themes::get(&str) -> Result<Theme, _>`, `HtmlMultiThemesBuilder::new().language(Language).themes(HashMap<String, Theme>).default_theme(&str).build() -> Result<HtmlMultiThemes, String>`, `HtmlMultiThemes: Formatter` with `format(&self, &str, &mut dyn Write) -> Result<()>`
- Produces: `fn highlight_code(lang: &str, code: &str) -> Option<String>` (private to render.rs) — `Some` with a complete `<pre class="lumis ...">` HTML snippet, `None` when lang is empty/PlainText/unknown or formatting fails. Verified output shape (from probe): `<pre class="lumis lumis-themes dark light" style="color:...; background-color:#ffffff; --lumis-dark:#e6edf3; --lumis-dark-bg:#0d1117;"><code class="language-rust" translate="no" tabindex="0"><div class="l-line" data-line="1"><span style="color:#cf222e; ... --lumis-dark:#ff7b72; ...">fn</span> ...</div></code></pre>`

- [ ] **Step 1: Add imports to `src/render.rs`**

Replace the top imports of `src/render.rs`:

```rust
use crate::Result;
use katex::{KatexContext, Settings, render_to_string};
use lumis::formatter::Formatter;
use lumis::formatters::html_multi_themes::HtmlMultiThemesBuilder;
use lumis::languages::Language;
use lumis::themes;
use pulldown_cmark::{Event, Options, Parser, html};
use std::{
    collections::HashMap,
    env,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
    time::Duration,
};
```

- [ ] **Step 2: Write the failing tests**

Append to the `#[cfg(test)] mod tests` block in `src/render.rs` (after `keeps_math_delimiters_literal_in_code`):

```rust
    #[test]
    fn highlights_fenced_rust_code() {
        let html = render_markdown("```rust\nfn main() {}\n```");

        assert!(html.contains("class=\"lumis lumis-themes dark light\""));
        assert!(html.contains("<code class=\"language-rust\" translate=\"no\""));
    }

    #[test]
    fn keeps_unmarked_code_blocks_plain() {
        let html = render_markdown("```\nplain text\n```");

        assert!(!html.contains("lumis"));
        assert!(html.contains("<pre><code>plain text"));
    }

    #[test]
    fn keeps_unknown_language_code_blocks_plain() {
        let html = render_markdown("```notalanguage\nwhatever\n```");

        assert!(!html.contains("lumis"));
        assert!(html.contains("<pre><code class=\"language-notalanguage\">whatever"));
    }

    #[test]
    fn keeps_plaintext_language_code_blocks_plain() {
        let html = render_markdown("```text\nplain\n```");

        assert!(!html.contains("lumis"));
    }
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test keeps_ 2>&1 | tail -5`
Expected: compile error `cannot find function highlight_code` (feature missing — correct RED).

- [ ] **Step 4: Write `highlight_code` and wire it into `render_markdown`**

Add before `render_markdown` in `src/render.rs`:

```rust
/// Syntax-highlights a fenced code block with lumis, returning `None` when the
/// language is unknown or formatting fails so the caller can fall back to a
/// plain code block.
fn highlight_code(lang: &str, code: &str) -> Option<String> {
    let language = Language::guess(Some(lang), "");
    if language == Language::PlainText {
        return None;
    }

    let mut theme_map = HashMap::new();
    theme_map.insert("light".to_string(), themes::get("github_light").ok()?);
    theme_map.insert("dark".to_string(), themes::get("github_dark").ok()?);

    let mut builder = HtmlMultiThemesBuilder::new();
    builder
        .language(language)
        .themes(theme_map)
        .default_theme("light");
    let formatter = builder.build().ok()?;

    let mut output = Vec::new();
    formatter.format(code, &mut output).ok()?;
    String::from_utf8(output).ok()
}
```

In `render_markdown`, add a `CodeBlock` arm to the existing event map (keep the math arms unchanged):

```rust
    let parser = Parser::new_ext(&markdown, options).map(|event| match event {
        Event::CodeBlock(lang, text) => {
            match highlight_code(lang.as_ref(), text.as_ref()) {
                Some(html) => Event::Html(html.into()),
                None => Event::CodeBlock(lang, text),
            }
        }
        Event::InlineMath(formula) => {
            Event::Html(render_math(&context, formula.as_ref(), &inline_settings).into())
        }
        Event::DisplayMath(formula) => {
            Event::Html(render_math(&context, formula.as_ref(), &display_settings).into())
        }
        event => event,
    });
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test highlights_ keeps_ 2>&1 | tail -5`
Expected: `test result: ok.` — all four new tests pass. If `keeps_plaintext_language_code_blocks_plain` fails (lumis recognizes `text` as a real language and produces output), keep the guard: `Language::guess` returning a non-PlainText for "text" means the language list is richer than assumed — the test then must be adjusted, but the `== Language::PlainText` guard itself is unchanged.

- [ ] **Step 6: Add the dark-theme CSS to `assets/reader.css`**

The lumis output hard-codes light-theme colors inline and exposes dark-theme values as `--lumis-dark*` CSS variables on the same elements. Append after the existing `.katex-error` block in `assets/reader.css`:

```css
@media (prefers-color-scheme: dark) {
  .lumis-themes.dark,
  .lumis-themes.dark span {
    color: var(--lumis-dark) !important;
    font-style: var(--lumis-dark-font-style) !important;
    font-weight: var(--lumis-dark-font-weight) !important;
    text-decoration: var(--lumis-dark-text-decoration) !important;
  }

  .lumis-themes.dark {
    background-color: var(--lumis-dark-bg) !important;
  }
}
```

- [ ] **Step 7: Update README feature list**

In `README.md`, extend the existing feature bullet:

```markdown
- Syntax-highlighted fenced code blocks (15 common languages) with light and dark themes, and one-click code copy
```

- [ ] **Step 8: Full check and commit**

Run: `cargo fmt && cargo test 2>&1 | tail -3 && cargo clippy --all-targets -- -D warnings 2>&1 | tail -1`
Expected: `test result: ok.` with all tests passing (36+), clippy clean.

```bash
git add Cargo.toml Cargo.lock src/render.rs assets/reader.css README.md
git commit -m "$(cat <<'EOF'
Highlight fenced code blocks with lumis

Render code blocks server-side through lumis (Tree-sitter) using
GitHub light and dark themes that follow prefers-color-scheme.
Unknown or missing languages fall back to the plain rendering.
Covers 15 common languages via per-language features to bound
compile time and binary size.

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>
EOF
)"
```

---
### Task 2: Copy button on code blocks

**Files:**
- Modify: `src/render.rs` (`COPY_SCRIPT` const, `render_document` script injection)
- Modify: `assets/reader.css` (copy button styles, `pre` positioning)
- Test: `src/render.rs` tests module

**Interfaces:**
- Consumes: `render_document(file: &Path, serve: bool) -> Result<String>` — existing function; `{script}` placeholder already exists in the HTML template (currently empty in preview mode, holds live-reload script in serve mode)
- Produces: `const COPY_SCRIPT: &str` — a `<script>...</script>` string injected in both preview and serve output

- [ ] **Step 1: Write the failing tests**

Append to the tests module in `src/render.rs`:

```rust
    #[test]
    fn both_modes_embed_the_copy_script_once() {
        let file = temp_markdown_file("hello");

        let preview = render_document(&file, false).unwrap();
        let served = render_document(&file, true).unwrap();

        assert_eq!(preview.matches("code-copy").count(), 1);
        assert_eq!(served.matches("code-copy").count(), 1);
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test both_modes_embed_the_copy_script_once 2>&1 | tail -4`
Expected: FAIL — `preview.matches("code-copy").count()` is 0.

- [ ] **Step 3: Add the copy script constant**

Add near `live_reload_script` in `src/render.rs`:

```rust
/// Adds a copy button to each code block. Buttons are created lazily on first
/// pointer hover so pages with no code blocks stay script-free of mutations.
const COPY_SCRIPT: &str = r#"<script>
(function () {
  document.addEventListener("pointerover", function (event) {
    var pre = event.target.closest("pre");
    if (!pre || pre.dataset.mdCopy) return;
    pre.dataset.mdCopy = "1";

    var button = document.createElement("button");
    button.type = "button";
    button.className = "code-copy";
    button.textContent = "Copy";
    button.setAttribute("aria-label", "Copy code to clipboard");
    button.addEventListener("click", function () {
      var text = pre.innerText;
      function copied() {
        button.textContent = "Copied ✓";
        setTimeout(function () { button.textContent = "Copy"; }, 1500);
      }
      if (navigator.clipboard && navigator.clipboard.writeText) {
        navigator.clipboard.writeText(text).then(copied, function () {});
      } else {
        var textarea = document.createElement("textarea");
        textarea.value = text;
        document.body.appendChild(textarea);
        textarea.select();
        try { document.execCommand("copy"); copied(); } catch (e) {}
        document.body.removeChild(textarea);
      }
    });
    pre.appendChild(button);
  });
})();
</script>"#;
```

- [ ] **Step 4: Inject the script in both modes**

In `render_document`, change the script selection (currently `let script = if serve { live_reload_script(file)? } else { String::new() };`) to:

```rust
    let script = if serve {
        format!("{COPY_SCRIPT}\n{}", live_reload_script(file)?)
    } else {
        COPY_SCRIPT.to_string()
    };
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test both_modes_embed_the_copy_script_once 2>&1 | tail -3`
Expected: PASS. Then `cargo test 2>&1 | tail -3` — all tests green (existing `preview_html_does_not_embed_live_reload_script` must still pass: `COPY_SCRIPT` contains neither `setInterval` nor `/refresh`).

- [ ] **Step 6: Add the button CSS to `assets/reader.css`**

Change the existing `pre` rule in `assets/reader.css` (add `position: relative;`):

```css
pre {
  position: relative;
  background: var(--pre-bg);
  border: 1px solid var(--border-strong);
  border-radius: 10px;
  color: var(--pre-ink);
  font-size: 0.86em;
  line-height: 1.62;
  overflow-x: auto;
  padding: 18px 20px;
  tab-size: 2;
}
```

Append after the lumis dark-theme block (from Task 1):

```css
.code-copy {
  position: absolute;
  top: 8px;
  right: 8px;
  align-items: center;
  background: var(--surface);
  border: 1px solid var(--border-strong);
  border-radius: 6px;
  color: var(--muted);
  cursor: pointer;
  font: 600 11px/1 ui-sans-serif, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
  opacity: 0;
  padding: 4px 9px;
  transition: opacity 140ms ease-out, color 140ms ease-out, border-color 140ms ease-out;
}

pre:hover .code-copy,
pre:focus-within .code-copy {
  opacity: 1;
}

.code-copy:hover {
  border-color: var(--accent);
  color: var(--accent);
}

.code-copy:focus-visible {
  outline: 3px solid var(--accent);
  outline-offset: 2px;
}
```

- [ ] **Step 7: Extend the CSS test and update README**

In the existing `reader_css_styles_katex_errors` test in `src/render.rs`, add an assertion:

```rust
    #[test]
    fn reader_css_styles_katex_errors() {
        let css = reader_css();

        assert!(css.contains(".katex-error"));
        assert!(css.contains(".code-copy"));
    }
```

Update the README feature bullet from Task 1 to include copy:

```markdown
- Syntax-highlighted fenced code blocks (15 common languages) with light and dark themes, and one-click code copy
```

(unchanged — the copy mention was included in Task 1's wording; verify it reads correctly and keep it.)

- [ ] **Step 8: Full check and commit**

Run: `cargo fmt && cargo test 2>&1 | tail -3 && cargo clippy --all-targets -- -D warnings 2>&1 | tail -1`
Expected: all tests pass (37+), clippy clean.

```bash
git add src/render.rs assets/reader.css README.md
git commit -m "$(cat <<'EOF'
Add copy buttons to code blocks

A small inline script adds a copy button to each code block on first
hover, visible on hover or keyboard focus. Uses the Clipboard API with
an execCommand fallback so previews opened from file:// still work.
The button reuses the reader's surface/border/accent variables and
follows the existing light and dark schemes.

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>
EOF
)"
```

---
### Task 3: End-to-end verification

**Files:** none (verification only)

- [ ] **Step 1: Full test suite**

Run: `cargo test 2>&1 | tail -3`
Expected: all tests pass.

- [ ] **Step 2: Smoke test preview mode**

```bash
mkdir -p /tmp/md-hl && printf '```rust\nfn main() { println!("hi"); }\n```\n\nplain text\n' > /tmp/md-hl/a.md
./target/debug/md-reader --no-open /tmp/md-hl/a.md
grep -c 'class="lumis lumis-themes dark light"' "$(ls -t /var/folders/*/*/T/md-reader/*.html 2>/dev/null | head -1)" 2>/dev/null || grep -c lumis /tmp/md-reader/*.html
rm -rf /tmp/md-hl
```

Expected: the preview HTML contains the lumis-wrapped highlighted block and the copy script. (The temp dir is platform-specific; adapt the path from the `Preview ...` line printed by md-reader.)

- [ ] **Step 3: Smoke test serve mode and dark theme in a real browser**

```bash
mkdir -p /tmp/md-hl && printf '# Hi\n\n```rust\nlet x = 1;\n```\n' > /tmp/md-hl/a.md
(./target/debug/md-reader --serve --no-open --port 8705 /tmp/md-hl/a.md &) && sleep 1
```

Load `http://127.0.0.1:8705/` with Playwright:
- assert the page contains `pre.lumis` with class `dark light`
- set `page.emulateMedia({ colorScheme: 'dark' })` (or use the MCP browser and check computed styles) and assert a highlighted span's computed `color` differs from the light-mode value (e.g. `rgb(255, 123, 114)` for `fn` in github_dark vs `rgb(207, 46, 46)` in light)
- hover over the code block, assert the copy button becomes visible; click it and assert the button text changes (Clipboard API may be restricted in the headless context — the execCommand fallback covers it)
- cleanup: kill the server, remove `/tmp/md-hl`

- [ ] **Step 4: Final commit if the browser test found fixes**

If Step 3 revealed issues, apply the fix (TDD: test first), then commit. Otherwise nothing to commit — the working tree is clean after Task 1 and Task 2 commits.
