use crate::Result;
use katex::{KatexContext, Settings, render_to_string};
use lumis::formatter::Formatter;
use lumis::formatters::html_multi_themes::HtmlMultiThemesBuilder;
use lumis::languages::Language;
use lumis::themes;
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd, html};
use std::{
    collections::HashMap,
    env,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
    time::Duration,
};

/// Previews are only read once by the browser, so anything older than this is
/// safe to remove when a new preview is written.
const PREVIEW_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

pub(crate) fn write_preview_file(source_file: &Path, html: &str) -> Result<PathBuf> {
    let preview_dir = env::temp_dir().join("md-reader");
    cleanup_stale_previews(&preview_dir, PREVIEW_MAX_AGE);
    fs::create_dir_all(&preview_dir)?;

    let stem = source_file
        .file_stem()
        .and_then(OsStr::to_str)
        .unwrap_or("preview");
    let output = preview_dir.join(format!(
        "{}-{}.html",
        sanitize_file_stem(stem),
        std::process::id()
    ));

    fs::write(&output, html)?;
    Ok(output)
}

/// Removes preview files in `preview_dir` that are older than `max_age`, so
/// repeated runs do not accumulate files in the temp directory. Best-effort:
/// anything that cannot be read or removed is left alone, and directories are
/// never removed.
fn cleanup_stale_previews(preview_dir: &Path, max_age: Duration) {
    let Ok(entries) = fs::read_dir(preview_dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(modified) = fs::metadata(&path).and_then(|meta| meta.modified()) else {
            continue;
        };
        let Ok(age) = modified.elapsed() else {
            continue;
        };

        if age > max_age && path.is_file() {
            let _ = fs::remove_file(&path);
        }
    }
}

pub(crate) fn render_document(file: &Path, serve: bool) -> Result<String> {
    let bytes = fs::read(file)?;
    let markdown = String::from_utf8_lossy(&bytes);
    let title = file
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("Markdown Reader");
    let body = render_markdown(&markdown);
    let script = if serve {
        format!("{COPY_SCRIPT}\n{}", live_reload_script(file)?)
    } else {
        COPY_SCRIPT.to_string()
    };
    let base_href = if serve {
        "/".to_string()
    } else {
        base_href_for(file)?
    };

    Ok(format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>{title}</title>
  <base href="{base_href}">
  <link rel="icon" href="data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 32 32'><rect width='32' height='32' rx='7' fill='%232563eb'/><text x='16' y='21' font-family='monospace' font-size='15' font-weight='700' fill='white' text-anchor='middle'>md</text></svg>">
  <style>{css}</style>
</head>
<body>
  <main class="reader">
    <header class="reader-header">
      <div class="reader-identity">
        <span class="reader-mark" aria-hidden="true">md</span>
        <span class="reader-label">md-reader</span>
      </div>
      <div class="file-name" title="{title}">{title}</div>
    </header>
    <article class="markdown-body">
      {body}
    </article>
  </main>
  {script}
</body>
</html>"#,
        title = escape_html(title),
        base_href = escape_html(&base_href),
        css = reader_css(),
        body = body,
        script = script
    ))
}

/// Adds a copy button to each code block. Buttons are created lazily on first
/// pointer hover so pages with no code blocks stay untouched.
const COPY_SCRIPT: &str = r#"<script>
(function () {
  document.addEventListener("pointerover", function (event) {
    var pre = event.target.closest("pre");
    if (!pre || pre.dataset.mdCopy) return;
    var code = pre.querySelector("code");
    if (!code) return;
    pre.dataset.mdCopy = "1";

    var button = document.createElement("button");
    button.type = "button";
    button.className = "code-copy";
    button.textContent = "Copy";
    button.setAttribute("aria-label", "Copy code to clipboard");
    button.addEventListener("click", function () {
      var text = code.innerText;
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

/// Polls the serve-mode `/refresh` endpoint and reloads the page when the
/// source file's modification time changes.
fn live_reload_script(file: &Path) -> Result<String> {
    let mtime = file_mtime_millis(file)?;
    Ok(format!(
        r#"<script>
(function () {{
  var current = "{mtime}";
  function poll() {{
    fetch("/refresh", {{ cache: "no-store" }})
      .then(function (response) {{ return response.ok ? response.text() : null; }})
      .then(function (text) {{
        if (text && text !== current) {{ location.reload(); }}
      }})
      .catch(function () {{}});
  }}
  setInterval(poll, 750);
}})();
</script>"#,
    ))
}

pub(crate) fn file_mtime_millis(file: &Path) -> Result<u128> {
    let modified = fs::metadata(file)?.modified()?;
    Ok(modified
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0))
}

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

fn render_markdown(markdown: &str) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_HEADING_ATTRIBUTES);
    options.insert(Options::ENABLE_MATH);

    let markdown = normalize_math_delimiters(markdown);
    let context = KatexContext::default();
    let inline_settings = Settings {
        throw_on_error: false,
        ..Settings::default()
    };
    let display_settings = Settings {
        display_mode: true,
        throw_on_error: false,
        ..Settings::default()
    };

    let parser = Parser::new_ext(&markdown, options).map(|event| match event {
        Event::InlineMath(formula) => {
            Event::Html(render_math(&context, formula.as_ref(), &inline_settings).into())
        }
        Event::DisplayMath(formula) => {
            Event::Html(render_math(&context, formula.as_ref(), &display_settings).into())
        }
        event => event,
    });

    // Collapse each code block into a single event: collect its text, highlight
    // it, and emit the highlighted HTML; when highlighting is not possible the
    // original Start/Text/End events are rebuilt unchanged.
    let mut iter = parser.peekable();
    let mut events = Vec::new();

    while let Some(event) = iter.next() {
        match event {
            Event::Start(Tag::CodeBlock(kind)) => {
                let mut text = String::new();
                for inner in iter.by_ref() {
                    match inner {
                        Event::Text(part) => text.push_str(part.as_ref()),
                        Event::End(TagEnd::CodeBlock) => break,
                        _ => {}
                    }
                }

                let lang = match &kind {
                    CodeBlockKind::Fenced(lang) => lang.as_ref(),
                    CodeBlockKind::Indented => "",
                };
                match highlight_code(lang, &text) {
                    Some(html) => events.push(Event::Html(html.into())),
                    None => {
                        events.push(Event::Start(Tag::CodeBlock(kind)));
                        events.push(Event::Text(text.into()));
                        events.push(Event::End(TagEnd::CodeBlock));
                    }
                }
            }
            event => events.push(event),
        }
    }

    let mut output = String::new();
    html::push_html(&mut output, events.into_iter());
    output
}

fn render_math(context: &KatexContext, formula: &str, settings: &Settings) -> String {
    render_to_string(context, formula, settings).unwrap_or_else(|error| {
        format!(
            r#"<span class="katex-error" title="{}">{}</span>"#,
            escape_html(&error.to_string()),
            escape_html(formula)
        )
    })
}

/// Converts the two common backslash delimiter pairs to the dollar delimiters
/// understood by pulldown-cmark. Code fences and inline code spans are left
/// unchanged so examples can still show the delimiters literally.
fn normalize_math_delimiters(markdown: &str) -> String {
    let mut output = String::with_capacity(markdown.len());
    let mut prose = String::new();
    let mut fence = None;

    for line in markdown.split_inclusive('\n') {
        let marker = code_fence_marker(line);

        match fence {
            Some((character, length)) => {
                output.push_str(line);
                if marker.is_some_and(|(candidate, candidate_length)| {
                    candidate == character && candidate_length >= length
                }) {
                    fence = None;
                }
            }
            None if marker.is_some() => {
                output.push_str(&normalize_latex_delimiters(&prose));
                prose.clear();
                output.push_str(line);
                fence = marker;
            }
            None => prose.push_str(line),
        }
    }

    output.push_str(&normalize_latex_delimiters(&prose));
    output
}

fn code_fence_marker(line: &str) -> Option<(u8, usize)> {
    let line = line.trim_start_matches([' ', '\t']);
    let marker = *line.as_bytes().first()?;
    if !matches!(marker, b'`' | b'~') {
        return None;
    }

    let length = line.bytes().take_while(|byte| *byte == marker).count();
    (length >= 3).then_some((marker, length))
}

fn normalize_latex_delimiters(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut position = 0;
    let mut inline_code_delimiter = None;

    while position < input.len() {
        let remaining = &input[position..];

        if remaining.starts_with('`') {
            let length = remaining.bytes().take_while(|byte| *byte == b'`').count();
            output.push_str(&remaining[..length]);
            position += length;

            match inline_code_delimiter {
                Some(opening_length) if opening_length == length => inline_code_delimiter = None,
                None => inline_code_delimiter = Some(length),
                _ => {}
            }
            continue;
        }

        if inline_code_delimiter.is_none()
            && remaining.starts_with(r"\[")
            && let Some(end) = find_latex_delimiter(input, position + 2, r"\]")
        {
            output.push_str("$$");
            output.push_str(&input[position + 2..end]);
            output.push_str("$$");
            position = end + 2;
            continue;
        }

        if inline_code_delimiter.is_none()
            && remaining.starts_with(r"\(")
            && let Some(end) = find_latex_delimiter(input, position + 2, r"\)")
        {
            output.push('$');
            output.push_str(&input[position + 2..end]);
            output.push('$');
            position = end + 2;
            continue;
        }

        let character = remaining.chars().next().expect("position is in bounds");
        output.push(character);
        position += character.len_utf8();
    }

    output
}

fn find_latex_delimiter(input: &str, mut position: usize, closing: &str) -> Option<usize> {
    let mut inline_code_delimiter = None;

    while position < input.len() {
        let remaining = &input[position..];

        if remaining.starts_with('`') {
            let length = remaining.bytes().take_while(|byte| *byte == b'`').count();
            position += length;
            match inline_code_delimiter {
                Some(opening_length) if opening_length == length => inline_code_delimiter = None,
                None => inline_code_delimiter = Some(length),
                _ => {}
            }
            continue;
        }

        if inline_code_delimiter.is_none() && remaining.starts_with(closing) {
            return Some(position);
        }

        position += remaining.chars().next()?.len_utf8();
    }

    None
}

fn base_href_for(file: &Path) -> Result<String> {
    let dir = file
        .parent()
        .ok_or_else(|| format!("could not find parent directory for {}", file.display()))?;
    path_to_file_url(dir)
}

fn path_to_file_url(path: &Path) -> Result<String> {
    let path = fs::canonicalize(path)?;
    let path = path.to_string_lossy().replace('\\', "/");
    let mut url = if cfg!(target_os = "windows") {
        format!("file:///{}", path.trim_start_matches('/'))
    } else {
        format!("file://{path}")
    };

    if !url.ends_with('/') {
        url.push('/');
    }

    Ok(percent_encode_url_path(&url))
}

fn percent_encode_url_path(input: &str) -> String {
    let mut output = String::new();

    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' | b':' => {
                output.push(byte as char)
            }
            _ => output.push_str(&format!("%{byte:02X}")),
        }
    }

    output
}

fn sanitize_file_stem(input: &str) -> String {
    let sanitized: String = input
        .chars()
        .map(|char| {
            if char.is_ascii_alphanumeric() || matches!(char, '-' | '_') {
                char
            } else {
                '-'
            }
        })
        .collect();

    let sanitized = sanitized.trim_matches('-');
    if sanitized.is_empty() {
        "preview".to_string()
    } else {
        sanitized.to_string()
    }
}

fn escape_html(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

const KATEX_CSS: &str = include_str!("../assets/katex/katex.min.css");

const KATEX_FONTS: &[(&str, &[u8])] = &[
    (
        "KaTeX_AMS-Regular.woff2",
        include_bytes!("../assets/katex/fonts/KaTeX_AMS-Regular.woff2"),
    ),
    (
        "KaTeX_Caligraphic-Bold.woff2",
        include_bytes!("../assets/katex/fonts/KaTeX_Caligraphic-Bold.woff2"),
    ),
    (
        "KaTeX_Caligraphic-Regular.woff2",
        include_bytes!("../assets/katex/fonts/KaTeX_Caligraphic-Regular.woff2"),
    ),
    (
        "KaTeX_Fraktur-Bold.woff2",
        include_bytes!("../assets/katex/fonts/KaTeX_Fraktur-Bold.woff2"),
    ),
    (
        "KaTeX_Fraktur-Regular.woff2",
        include_bytes!("../assets/katex/fonts/KaTeX_Fraktur-Regular.woff2"),
    ),
    (
        "KaTeX_Main-Bold.woff2",
        include_bytes!("../assets/katex/fonts/KaTeX_Main-Bold.woff2"),
    ),
    (
        "KaTeX_Main-BoldItalic.woff2",
        include_bytes!("../assets/katex/fonts/KaTeX_Main-BoldItalic.woff2"),
    ),
    (
        "KaTeX_Main-Italic.woff2",
        include_bytes!("../assets/katex/fonts/KaTeX_Main-Italic.woff2"),
    ),
    (
        "KaTeX_Main-Regular.woff2",
        include_bytes!("../assets/katex/fonts/KaTeX_Main-Regular.woff2"),
    ),
    (
        "KaTeX_Math-BoldItalic.woff2",
        include_bytes!("../assets/katex/fonts/KaTeX_Math-BoldItalic.woff2"),
    ),
    (
        "KaTeX_Math-Italic.woff2",
        include_bytes!("../assets/katex/fonts/KaTeX_Math-Italic.woff2"),
    ),
    (
        "KaTeX_SansSerif-Bold.woff2",
        include_bytes!("../assets/katex/fonts/KaTeX_SansSerif-Bold.woff2"),
    ),
    (
        "KaTeX_SansSerif-Italic.woff2",
        include_bytes!("../assets/katex/fonts/KaTeX_SansSerif-Italic.woff2"),
    ),
    (
        "KaTeX_SansSerif-Regular.woff2",
        include_bytes!("../assets/katex/fonts/KaTeX_SansSerif-Regular.woff2"),
    ),
    (
        "KaTeX_Script-Regular.woff2",
        include_bytes!("../assets/katex/fonts/KaTeX_Script-Regular.woff2"),
    ),
    (
        "KaTeX_Size1-Regular.woff2",
        include_bytes!("../assets/katex/fonts/KaTeX_Size1-Regular.woff2"),
    ),
    (
        "KaTeX_Size2-Regular.woff2",
        include_bytes!("../assets/katex/fonts/KaTeX_Size2-Regular.woff2"),
    ),
    (
        "KaTeX_Size3-Regular.woff2",
        include_bytes!("../assets/katex/fonts/KaTeX_Size3-Regular.woff2"),
    ),
    (
        "KaTeX_Size4-Regular.woff2",
        include_bytes!("../assets/katex/fonts/KaTeX_Size4-Regular.woff2"),
    ),
    (
        "KaTeX_Typewriter-Regular.woff2",
        include_bytes!("../assets/katex/fonts/KaTeX_Typewriter-Regular.woff2"),
    ),
];

fn reader_css() -> &'static str {
    static CSS: OnceLock<String> = OnceLock::new();

    CSS.get_or_init(|| {
        let mut katex_css = KATEX_CSS.to_string();
        for (filename, data) in KATEX_FONTS {
            let source = format!("url(fonts/{filename})");
            let embedded = format!("url(data:font/woff2;base64,{})", base64_encode(data));
            katex_css = katex_css.replace(&source, &embedded);

            let stem = filename
                .strip_suffix(".woff2")
                .expect("all KaTeX fonts are woff2");
            let fallbacks = format!(
                r#",url(fonts/{stem}.woff) format("woff"),url(fonts/{stem}.ttf) format("truetype")"#
            );
            katex_css = katex_css.replace(&fallbacks, "");
        }

        format!("{katex_css}\n{READER_CSS}")
    })
}

fn base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut output = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or_default();
        let third = chunk.get(2).copied().unwrap_or_default();

        output.push(ALPHABET[(first >> 2) as usize] as char);
        output.push(ALPHABET[((first & 0b0000_0011) << 4 | second >> 4) as usize] as char);
        output.push(if chunk.len() > 1 {
            ALPHABET[((second & 0b0000_1111) << 2 | third >> 6) as usize] as char
        } else {
            '='
        });
        output.push(if chunk.len() > 2 {
            ALPHABET[(third & 0b0011_1111) as usize] as char
        } else {
            '='
        });
    }

    output
}

const READER_CSS: &str = include_str!("../assets/reader.css");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::*;

    #[test]
    fn renders_common_markdown_extensions() {
        let html = render_markdown("| A | B |\n| - | - |\n| 1 | 2 |\n\n- [x] done\n\n~~old~~");

        assert!(html.contains("<table>"));
        assert!(html.contains("checked"));
        assert!(html.contains("<del>old</del>"));
    }

    #[test]
    fn renders_latex_math_with_backslash_and_dollar_delimiters() {
        let html = render_markdown(
            r"The check digit is \(a_{18}\).

\[
(a_{18} + a_{17} \cdot 2 + a_1 \cdot 2^{17}) \bmod 11 = 1
\]

And $x^2$ is inline.",
        );

        assert!(html.contains("class=\"katex\""));
        assert!(html.contains("katex-display"));
        assert!(!html.contains(r"\["));
        assert!(!html.contains(r"\("));
    }

    #[test]
    fn keeps_math_delimiters_literal_in_code_spans_of_any_length() {
        let html = render_markdown("`\\(a_i\\)` and ``\\(b_i\\)``");

        assert!(html.contains("<code>\\(a_i\\)</code>"));
        assert!(html.contains("<code>\\(b_i\\)</code>"));
    }

    #[test]
    fn keeps_math_delimiters_literal_inside_tilde_fences() {
        let html = render_markdown("~~~tex\n\\[a_i\\]\n~~~");

        assert!(html.contains("<code class=\"language-tex\">\\[a_i\\]"));
    }

    #[test]
    fn leaves_unmatched_backslash_delimiters_literal() {
        let html = render_markdown(r"open \( without a closer");

        assert!(!html.contains("class=\"katex\""));
        // `\(` is a CommonMark escape for a literal parenthesis, so the
        // backslash disappears and no math is rendered.
        assert!(html.contains("open ( without a closer"));
    }

    #[test]
    fn keeps_math_delimiters_literal_in_code() {
        let html = render_markdown("`\\(a_i\\)`\n\n```tex\n\\[a_i\\]\n```");

        assert!(html.contains("<code>\\(a_i\\)</code>"));
        assert!(html.contains("<code class=\"language-tex\">\\[a_i\\]"));
    }

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

    #[test]
    fn embeds_katex_fonts_for_offline_previews() {
        let css = reader_css();

        assert!(css.contains("data:font/woff2;base64,"));
        assert!(!css.contains("url(fonts/"));
    }

    #[test]
    fn encodes_base64() {
        assert_eq!(base64_encode(b"Man"), "TWFu");
        assert_eq!(base64_encode(b"Ma"), "TWE=");
        assert_eq!(base64_encode(b"M"), "TQ==");
    }

    #[test]
    fn escapes_html_for_titles() {
        assert_eq!(escape_html("a&b<c>\"'"), "a&amp;b&lt;c&gt;&quot;&#39;");
    }

    #[test]
    fn sanitizes_preview_file_stems() {
        assert_eq!(sanitize_file_stem("daily notes"), "daily-notes");
        assert_eq!(sanitize_file_stem("!!!"), "preview");
    }

    #[test]
    fn percent_encodes_file_urls() {
        assert_eq!(
            percent_encode_url_path("file:///tmp/my notes/"),
            "file:///tmp/my%20notes/"
        );
    }

    #[test]
    fn serve_html_embeds_live_reload_script() {
        let file = temp_markdown_file("hello");
        let mtime = file_mtime_millis(&file).unwrap();
        let html = render_document(&file, true).unwrap();

        assert!(html.contains("setInterval"));
        assert!(html.contains("/refresh"));
        assert!(html.contains(&format!("var current = \"{mtime}\"")));
    }

    #[test]
    fn preview_html_does_not_embed_live_reload_script() {
        let file = temp_markdown_file("hello");
        let html = render_document(&file, false).unwrap();

        assert!(!html.contains("setInterval"));
        assert!(!html.contains("/refresh"));
    }

    #[test]
    fn serve_html_uses_server_root_as_base_href() {
        let file = temp_markdown_file("hello");
        let html = render_document(&file, true).unwrap();

        assert!(html.contains("<base href=\"/\">"));
    }

    #[test]
    fn preview_html_uses_file_url_as_base_href() {
        let file = temp_markdown_file("hello");
        let html = render_document(&file, false).unwrap();

        assert!(html.contains("<base href=\"file://"));
    }

    #[test]
    fn html_includes_an_inline_favicon() {
        let file = temp_markdown_file("hello");
        let html = render_document(&file, false).unwrap();

        assert!(html.contains("rel=\"icon\""));
        assert!(html.contains("data:image/svg+xml"));
    }

    #[test]
    fn both_modes_embed_the_copy_script_once() {
        let file = temp_markdown_file("hello");

        let preview = render_document(&file, false).unwrap();
        let served = render_document(&file, true).unwrap();

        assert_eq!(preview.matches("Copied ✓").count(), 1);
        assert_eq!(served.matches("Copied ✓").count(), 1);
    }

    #[test]
    fn copy_script_excludes_the_copy_button_text() {
        assert!(COPY_SCRIPT.contains(r#"var text = code.innerText;"#));
        assert!(!COPY_SCRIPT.contains(r#"var text = pre.innerText;"#));
    }

    #[test]
    fn reader_css_styles_katex_errors() {
        let css = reader_css();

        assert!(css.contains(".katex-error"));
        assert!(css.contains(".code-copy"));
    }

    #[test]
    fn renders_files_with_invalid_utf8_lossily() {
        let dir = temp_dir("md-reader-binary");
        let path = dir.join("binary.md");
        fs::write(&path, b"plain \xff\xfe bytes").unwrap();

        let html = render_document(&path, false).unwrap();

        assert!(html.contains('\u{FFFD}'));
    }

    #[test]
    fn cleanup_removes_stale_preview_files_but_keeps_fresh_ones() {
        let dir = temp_dir("md-reader-cleanup");
        let stale = dir.join("stale.html");
        let fresh = dir.join("fresh.html");
        fs::write(&stale, "old").unwrap();
        fs::write(&fresh, "new").unwrap();
        backdate(&stale, Duration::from_secs(48 * 3600));

        cleanup_stale_previews(&dir, Duration::from_secs(24 * 3600));

        assert!(!stale.exists());
        assert!(fresh.exists());
    }

    #[test]
    fn cleanup_only_removes_files_not_directories() {
        let dir = temp_dir("md-reader-cleanup");
        fs::create_dir_all(dir.join("subdir")).unwrap();

        cleanup_stale_previews(&dir, Duration::ZERO);

        assert!(dir.join("subdir").is_dir());
    }
}
