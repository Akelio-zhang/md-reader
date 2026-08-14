use katex::{KatexContext, Settings, render_to_string};
use pulldown_cmark::{Event, Options, Parser, html};
use std::{
    env,
    error::Error,
    ffi::OsStr,
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Config {
    file: PathBuf,
    host: String,
    port: u16,
    open_browser: bool,
    mode: Mode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Preview,
    Serve,
}

impl Config {
    fn parse<I, S>(args: I) -> Result<Self>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut args = args.into_iter().map(Into::into);
        let _program = args.next();

        let mut file = None;
        let mut host = "127.0.0.1".to_string();
        let mut port = 0;
        let mut open_browser = true;
        let mut mode = Mode::Preview;

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "-h" | "--help" => return Err(usage().into()),
                "--no-open" => open_browser = false,
                "--serve" => mode = Mode::Serve,
                "--host" => {
                    host = args
                        .next()
                        .ok_or_else(|| "--host requires a value".to_string())?;
                }
                "--port" => {
                    let value = args
                        .next()
                        .ok_or_else(|| "--port requires a value".to_string())?;
                    port = value.parse::<u16>()?;
                }
                _ if arg.starts_with("--host=") => {
                    host = arg["--host=".len()..].to_string();
                }
                _ if arg.starts_with("--port=") => {
                    port = arg["--port=".len()..].parse::<u16>()?;
                }
                _ if arg.starts_with('-') => {
                    return Err(format!("unknown option: {arg}\n\n{}", usage()).into());
                }
                _ => {
                    if file.replace(PathBuf::from(arg)).is_some() {
                        return Err(
                            format!("only one markdown file can be opened\n\n{}", usage()).into(),
                        );
                    }
                }
            }
        }

        let file = file.ok_or_else(usage)?;

        Ok(Self {
            file,
            host,
            port,
            open_browser,
            mode,
        })
    }
}

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    if args
        .iter()
        .skip(1)
        .any(|arg| matches!(arg.as_str(), "-h" | "--help"))
    {
        println!("{}", usage());
        return Ok(());
    }

    let config = Config::parse(args)?;
    let file = fs::canonicalize(&config.file)?;

    if !file.is_file() {
        return Err(format!("not a file: {}", file.display()).into());
    }

    match config.mode {
        Mode::Preview => preview_file(&file, config.open_browser),
        Mode::Serve => serve_file(&file, &config),
    }
}

fn preview_file(file: &Path, open_browser: bool) -> Result<()> {
    let html = render_document(file, false)?;
    let preview_file = write_preview_file(file, &html)?;

    println!("Rendered {}", file.display());
    println!("Preview {}", preview_file.display());

    if open_browser {
        open_in_browser(&preview_file)?;
    }

    Ok(())
}

fn serve_file(file: &Path, config: &Config) -> Result<()> {
    let listener = TcpListener::bind((config.host.as_str(), config.port))?;
    let addr = listener.local_addr()?;
    let url = format!("http://{}:{}/", config.host, addr.port());

    println!("Reading {}", file.display());
    println!("Serving {url}");
    println!("Press Ctrl-C to stop.");

    if config.open_browser {
        open_url(&url)?;
    }

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                if let Err(err) = handle_connection(stream, file) {
                    eprintln!("request failed: {err}");
                }
            }
            Err(err) => eprintln!("connection failed: {err}"),
        }
    }

    Ok(())
}

fn write_preview_file(source_file: &Path, html: &str) -> Result<PathBuf> {
    let preview_dir = env::temp_dir().join("md-reader");
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

struct Response {
    status: &'static str,
    content_type: &'static str,
    body: Vec<u8>,
}

fn handle_connection(mut stream: TcpStream, file: &Path) -> Result<()> {
    let mut buffer = [0; 2048];
    let bytes_read = stream.read(&mut buffer)?;
    let request = String::from_utf8_lossy(&buffer[..bytes_read]);
    let request_line = request.lines().next().unwrap_or_default();
    let path = request_line
        .split_whitespace()
        .nth(1)
        .unwrap_or("/")
        .split('?')
        .next()
        .unwrap_or("/");

    respond(&mut stream, &handle_request(path, file)?)
}

fn handle_request(path: &str, file: &Path) -> Result<Response> {
    match path {
        "/" | "/index.html" => Ok(Response {
            status: "200 OK",
            content_type: "text/html; charset=utf-8",
            body: render_document(file, true)?.into_bytes(),
        }),
        "/refresh" => Ok(Response {
            status: "200 OK",
            content_type: "text/plain; charset=utf-8",
            body: file_mtime_millis(file)?.to_string().into_bytes(),
        }),
        "/health" => Ok(Response {
            status: "200 OK",
            content_type: "text/plain; charset=utf-8",
            body: b"ok".to_vec(),
        }),
        _ => Ok(Response {
            status: "404 Not Found",
            content_type: "text/plain; charset=utf-8",
            body: b"not found".to_vec(),
        }),
    }
}

fn respond(stream: &mut TcpStream, response: &Response) -> Result<()> {
    write!(
        stream,
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response.status, response.content_type, response.body.len()
    )?;
    stream.write_all(&response.body)?;
    stream.flush()?;
    Ok(())
}

fn render_document(file: &Path, serve: bool) -> Result<String> {
    let markdown = fs::read_to_string(file)?;
    let title = file
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("Markdown Reader");
    let body = render_markdown(&markdown);
    let script = if serve {
        live_reload_script(file)?
    } else {
        String::new()
    };

    Ok(format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>{title}</title>
  <base href="{base_href}">
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
        base_href = escape_html(&base_href_for(file)?),
        css = reader_css(),
        body = body,
        script = script
    ))
}

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

fn file_mtime_millis(file: &Path) -> Result<u128> {
    let modified = fs::metadata(file)?.modified()?;
    Ok(modified
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0))
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
    let mut output = String::new();
    html::push_html(&mut output, parser);
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

fn open_in_browser(path: &Path) -> Result<()> {
    let target = path.to_string_lossy();
    let status = if cfg!(target_os = "macos") {
        Command::new("open").arg(path).status()?
    } else if cfg!(target_os = "windows") {
        Command::new("cmd")
            .args(["/C", "start", "", target.as_ref()])
            .status()?
    } else {
        Command::new("xdg-open").arg(path).status()?
    };

    if status.success() {
        Ok(())
    } else {
        Err(format!("failed to open browser for {}", path.display()).into())
    }
}

fn open_url(url: &str) -> Result<()> {
    let status = if cfg!(target_os = "macos") {
        Command::new("open").arg(url).status()?
    } else if cfg!(target_os = "windows") {
        Command::new("cmd")
            .args(["/C", "start", "", url])
            .status()?
    } else {
        Command::new("xdg-open").arg(url).status()?
    };

    if status.success() {
        Ok(())
    } else {
        Err(format!("failed to open browser for {url}").into())
    }
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

fn usage() -> String {
    "Usage: md-reader [OPTIONS] <file.md>\n\nOptions:\n  --serve          Serve the file over local HTTP instead of writing a temp HTML preview\n  --host <host>    Host to bind in --serve mode (default: 127.0.0.1)\n  --port <port>    Port to bind in --serve mode (default: random available port)\n  --no-open        Render or serve without opening a browser\n  -h, --help       Show this help".to_string()
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

const READER_CSS: &str = r#"
:root {
  color-scheme: light dark;
  --canvas: #f6f7fb;
  --surface: #ffffff;
  --ink: #1d2939;
  --muted: #66758a;
  --border: #e3e8ef;
  --border-strong: #d7dfe9;
  --accent: #2563eb;
  --accent-hover: #1d4ed8;
  --accent-subtle: #eff6ff;
  --code-bg: #f5f7fa;
  --pre-bg: #141c2b;
  --pre-ink: #edf2f9;
  --quote-bg: #f8fafc;
  --table-alt: #fafbfc;
  --selection: #dbeafe;
  --shadow: 0 1px 2px rgb(16 24 40 / 0.02), 0 12px 32px rgb(16 24 40 / 0.05);
}

@media (prefers-color-scheme: dark) {
  :root {
    --canvas: #0b1020;
    --surface: #111827;
    --ink: #edf2f7;
    --muted: #a7b4c7;
    --border: #263248;
    --border-strong: #34425b;
    --accent: #7db3ff;
    --accent-hover: #a7caff;
    --accent-subtle: #14223a;
    --code-bg: #182236;
    --pre-bg: #090e1a;
    --pre-ink: #e6edf8;
    --quote-bg: #121d2f;
    --table-alt: #151f31;
    --selection: #2d4d7b;
    --shadow: 0 1px 2px rgb(0 0 0 / 0.2), 0 18px 42px rgb(0 0 0 / 0.2);
  }
}

* {
  box-sizing: border-box;
}

html {
  background: var(--canvas);
  overflow-x: clip;
  text-size-adjust: 100%;
}

body {
  margin: 0;
  min-height: 100vh;
  background: radial-gradient(64rem 36rem at 50% -10rem, rgb(255 255 255 / 0.92), transparent 72%), var(--canvas);
  color: var(--ink);
  font-family: ui-sans-serif, -apple-system, BlinkMacSystemFont, "Segoe UI", "PingFang SC", "Hiragino Sans GB", "Microsoft YaHei", "Noto Sans", Helvetica, Arial, sans-serif;
  font-size: 17px;
  line-height: 1.72;
  max-width: 100%;
  overflow-x: clip;
  overflow-wrap: break-word;
  -webkit-font-smoothing: antialiased;
  text-rendering: optimizeLegibility;
}

::selection {
  background: var(--selection);
}

:focus-visible {
  outline: 3px solid var(--accent);
  outline-offset: 3px;
}

.reader {
  min-width: 0;
  width: 100%;
  max-width: 960px;
  margin: 0 auto;
  padding: clamp(24px, 5vw, 56px) clamp(16px, 4vw, 40px) 64px;
}

.reader-header {
  align-items: center;
  display: flex;
  gap: 16px;
  justify-content: space-between;
  margin: 0 4px 16px;
  min-height: 28px;
}

.reader-identity {
  align-items: center;
  color: var(--muted);
  display: flex;
  flex: 0 0 auto;
  font-size: 12px;
  font-weight: 650;
  gap: 8px;
  letter-spacing: 0.01em;
}

.reader-mark {
  align-items: center;
  background: var(--accent-subtle);
  border: 1px solid var(--border);
  border-radius: 7px;
  color: var(--accent);
  display: inline-flex;
  font-family: ui-monospace, SFMono-Regular, SF Mono, Menlo, Consolas, monospace;
  font-size: 10px;
  font-weight: 700;
  height: 22px;
  justify-content: center;
  letter-spacing: -0.06em;
  width: 25px;
}

.file-name {
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 999px;
  color: var(--muted);
  font-family: ui-monospace, SFMono-Regular, SF Mono, Menlo, Consolas, monospace;
  font-size: 12px;
  line-height: 1.25;
  max-width: min(66%, 440px);
  overflow: hidden;
  padding: 5px 10px;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.markdown-body {
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 14px;
  box-shadow: var(--shadow);
  min-width: 0;
  max-width: 100%;
  overflow-x: hidden;
  padding: clamp(26px, 5vw, 52px);
  width: 100%;
}

.markdown-body > :first-child {
  margin-top: 0;
}

.markdown-body > :last-child {
  margin-bottom: 0;
}

h1, h2, h3, h4, h5, h6 {
  color: var(--ink);
  font-weight: 650;
  letter-spacing: -0.025em;
  line-height: 1.22;
  margin: 44px 0 18px;
  scroll-margin-top: 24px;
}

h1 {
  border: 0;
  font-size: clamp(2rem, 4vw, 2.65rem);
  letter-spacing: -0.045em;
  margin-bottom: 26px;
  padding: 0;
  text-wrap: balance;
}

h2 {
  border-bottom: 1px solid var(--border);
  font-size: clamp(1.45rem, 2.6vw, 1.72rem);
  padding-bottom: 0.45em;
  text-wrap: balance;
}

h3 {
  font-size: 1.25rem;
}

h4 {
  font-size: 1.05rem;
}

h5 {
  font-size: 0.95rem;
}

h6 {
  color: var(--muted);
  font-size: 0.85rem;
}

p, ul, ol, blockquote, pre, table, .katex-display {
  margin-bottom: 22px;
  margin-top: 0;
}

p {
  max-width: 74ch;
}

ul, ol {
  padding-left: 1.55em;
}

li + li {
  margin-top: 0.4em;
}

li > p {
  margin-top: 18px;
}

a {
  color: var(--accent);
  text-decoration-color: rgb(37 99 235 / 0.34);
  text-decoration-thickness: 0.08em;
  text-underline-offset: 0.16em;
  transition: color 140ms ease-out, text-decoration-color 140ms ease-out;
}

@media (hover: hover) and (pointer: fine) {
  a:hover {
    color: var(--accent-hover);
    text-decoration-color: currentColor;
  }
}

blockquote {
  background: var(--quote-bg);
  border: 1px solid var(--border);
  border-left: 3px solid var(--accent);
  border-radius: 0 8px 8px 0;
  color: var(--muted);
  margin-left: 0;
  padding: 12px 16px;
}

blockquote > :last-child {
  margin-bottom: 0;
}

code, pre {
  font-family: ui-monospace, SFMono-Regular, SF Mono, Menlo, Consolas, "Liberation Mono", monospace;
}

code {
  background: var(--code-bg);
  border: 1px solid var(--border);
  border-radius: 5px;
  font-size: 0.84em;
  margin: 0;
  padding: 0.15em 0.38em;
}

pre {
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

pre code {
  background: transparent;
  border: 0;
  color: inherit;
  padding: 0;
}

table {
  border: 1px solid var(--border);
  border-collapse: separate;
  border-radius: 10px;
  border-spacing: 0;
  display: block;
  font-size: 0.94em;
  max-width: 100%;
  overflow-x: auto;
  width: fit-content;
  min-width: min(100%, 480px);
}

th, td {
  border-bottom: 1px solid var(--border);
  border-right: 1px solid var(--border);
  padding: 9px 13px;
  text-align: left;
}

th {
  background: var(--code-bg);
  font-weight: 650;
}

th:last-child, td:last-child {
  border-right: 0;
}

tr:last-child td {
  border-bottom: 0;
}

tr:nth-child(2n) {
  background: var(--table-alt);
}

img {
  border: 1px solid var(--border);
  border-radius: 10px;
  box-shadow: 0 1px 2px rgb(16 24 40 / 0.06);
  height: auto;
  max-width: 100%;
}

input[type="checkbox"] {
  accent-color: var(--accent);
  margin: 0 0.2em 0.25em -1.4em;
  vertical-align: middle;
}

hr {
  background: var(--border);
  border: 0;
  height: 1px;
  margin: 40px 0;
  padding: 0;
}

.katex-display {
  display: block;
  max-width: 100%;
  overflow-x: auto;
  padding: 0.35rem 0.2rem;
  scrollbar-color: var(--border-strong) transparent;
  width: 100%;
}

.katex-display > .katex {
  min-width: 0;
}

@media (prefers-color-scheme: dark) {
  body {
    background: radial-gradient(64rem 36rem at 50% -10rem, rgb(37 57 92 / 0.38), transparent 72%), var(--canvas);
  }
}

@media (prefers-reduced-motion: reduce) {
  *, *::before, *::after {
    scroll-behavior: auto !important;
    transition-duration: 0.01ms !important;
  }
}

@media (max-width: 650px) {
  body {
    font-size: 16px;
    line-height: 1.68;
  }

  .reader {
    padding: 24px 16px 40px;
  }

  .reader-header {
    margin-bottom: 14px;
  }

  .reader-label {
    display: none;
  }

  .file-name {
    max-width: calc(100% - 42px);
  }

  .markdown-body {
    border-radius: 11px;
    padding: 26px 20px;
  }

  h1 {
    font-size: 2rem;
  }

  h1, h2, h3, h4, h5, h6 {
    margin-top: 36px;
  }

  p, ul, ol, blockquote, pre, table, .katex-display {
    margin-bottom: 19px;
  }

  pre {
    border-radius: 8px;
    margin-left: -4px;
    margin-right: -4px;
    padding: 16px;
  }

  .katex-display {
    margin-left: -4px;
    margin-right: -4px;
    text-align: left;
  }

  .katex-display > .katex {
    text-align: left;
  }
}

@media print {
  :root {
    color-scheme: light;
  }

  body {
    background: #ffffff;
    color: #111827;
  }

  .reader {
    max-width: none;
    padding: 0;
  }

  .reader-header {
    display: none;
  }

  .markdown-body {
    border: 0;
    box-shadow: none;
    padding: 0;
  }
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static PREVIEW_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn temp_markdown_file(contents: &str) -> PathBuf {
        let n = PREVIEW_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join("md-reader-tests");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("notes-{}-{n}.md", std::process::id()));
        fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn parses_required_file_and_defaults() {
        let config = Config::parse(["md-reader", "notes.md"]).unwrap();

        assert_eq!(config.file, PathBuf::from("notes.md"));
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 0);
        assert!(config.open_browser);
        assert_eq!(config.mode, Mode::Preview);
    }

    #[test]
    fn parses_options() {
        let config = Config::parse([
            "md-reader",
            "--host",
            "localhost",
            "--port=4000",
            "--no-open",
            "--serve",
            "notes.md",
        ])
        .unwrap();

        assert_eq!(config.host, "localhost");
        assert_eq!(config.port, 4000);
        assert!(!config.open_browser);
        assert_eq!(config.mode, Mode::Serve);
    }

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
    fn keeps_math_delimiters_literal_in_code() {
        let html = render_markdown("`\\(a_i\\)`\n\n```tex\n\\[a_i\\]\n```");

        assert!(html.contains("<code>\\(a_i\\)</code>"));
        assert!(html.contains("<code class=\"language-tex\">\\[a_i\\]"));
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
    fn refresh_route_reports_file_mtime_in_milliseconds() {
        let file = temp_markdown_file("hello");
        let response = handle_request("/refresh", &file).unwrap();

        assert_eq!(response.status, "200 OK");
        assert_eq!(response.content_type, "text/plain; charset=utf-8");
        assert_eq!(
            String::from_utf8(response.body).unwrap(),
            file_mtime_millis(&file).unwrap().to_string()
        );
    }

    #[test]
    fn serve_root_route_returns_rendered_document() {
        let file = temp_markdown_file("# Hello");
        let response = handle_request("/", &file).unwrap();

        assert_eq!(response.status, "200 OK");
        assert_eq!(response.content_type, "text/html; charset=utf-8");
        assert!(String::from_utf8(response.body).unwrap().contains("<h1>Hello</h1>"));
    }
}
