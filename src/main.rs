use pulldown_cmark::{Options, Parser, html};
use std::{
    env,
    error::Error,
    ffi::OsStr,
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::Command,
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
    let html = render_document(file)?;
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
                if let Err(err) = handle_connection(stream, &file) {
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

    match path {
        "/" | "/index.html" => respond(
            &mut stream,
            "200 OK",
            "text/html; charset=utf-8",
            render_document(file)?.as_bytes(),
        )?,
        "/health" => respond(&mut stream, "200 OK", "text/plain; charset=utf-8", b"ok")?,
        _ => respond(
            &mut stream,
            "404 Not Found",
            "text/plain; charset=utf-8",
            b"not found",
        )?,
    }

    Ok(())
}

fn respond(stream: &mut TcpStream, status: &str, content_type: &str, body: &[u8]) -> Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)?;
    stream.flush()?;
    Ok(())
}

fn render_document(file: &Path) -> Result<String> {
    let markdown = fs::read_to_string(file)?;
    let title = file
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("Markdown Reader");
    let body = render_markdown(&markdown);

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
      <div class="file-name">{title}</div>
    </header>
    <article class="markdown-body">
      {body}
    </article>
  </main>
</body>
</html>"#,
        title = escape_html(title),
        base_href = escape_html(&base_href_for(file)?),
        css = reader_css(),
        body = body
    ))
}

fn render_markdown(markdown: &str) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_HEADING_ATTRIBUTES);

    let parser = Parser::new_ext(markdown, options);
    let mut output = String::new();
    html::push_html(&mut output, parser);
    output
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

fn reader_css() -> &'static str {
    r#"
:root {
  color-scheme: light dark;
  --bg: #ffffff;
  --ink: #1f2328;
  --muted: #59636e;
  --panel: #ffffff;
  --border: #d1d9e0;
  --border-muted: #d8dee4;
  --accent: #0969da;
  --code-bg: #f6f8fa;
  --table-alt: #f6f8fa;
}

@media (prefers-color-scheme: dark) {
  :root {
    --bg: #0d1117;
    --ink: #f0f6fc;
    --muted: #9198a1;
    --panel: #0d1117;
    --border: #3d444d;
    --border-muted: #3d444d;
    --accent: #4493f8;
    --code-bg: #151b23;
    --table-alt: #151b23;
  }
}

* {
  box-sizing: border-box;
}

body {
  margin: 0;
  background: var(--bg);
  color: var(--ink);
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", "Noto Sans", Helvetica, Arial, sans-serif;
  font-size: 16px;
  line-height: 1.5;
  word-wrap: break-word;
}

.reader {
  width: min(100%, 1012px);
  margin: 0 auto;
  padding: 32px;
}

.reader-header {
  margin-bottom: 16px;
  padding-bottom: 8px;
  border-bottom: 1px solid var(--border);
}

.file-name {
  color: var(--muted);
  font-family: inherit;
  font-size: 12px;
  font-weight: 600;
}

.markdown-body {
  background: var(--panel);
  border: 1px solid var(--border-muted);
  border-radius: 6px;
  padding: 32px;
}

.markdown-body > :first-child {
  margin-top: 0;
}

.markdown-body > :last-child {
  margin-bottom: 0;
}

h1, h2, h3, h4, h5, h6 {
  font-weight: 600;
  line-height: 1.25;
  margin: 24px 0 16px;
}

h1 {
  border-bottom: 1px solid var(--border-muted);
  font-size: 2em;
  padding-bottom: 0.3em;
}

h2 {
  border-bottom: 1px solid var(--border-muted);
  font-size: 1.5em;
  padding-bottom: 0.3em;
}

h3 {
  font-size: 1.25em;
}

h4 {
  font-size: 1em;
}

h5 {
  font-size: 0.875em;
}

h6 {
  color: var(--muted);
  font-size: 0.85em;
}

p, ul, ol, blockquote, pre, table {
  margin-bottom: 16px;
  margin-top: 0;
}

ul, ol {
  padding-left: 2em;
}

li + li {
  margin-top: 0.25em;
}

li > p {
  margin-top: 16px;
}

a {
  color: var(--accent);
  text-decoration: none;
}

a:hover {
  text-decoration: underline;
}

blockquote {
  border-left: 0.25em solid var(--border);
  color: var(--muted);
  margin-left: 0;
  padding: 0 1em;
}

code, pre {
  font-family: ui-monospace, SFMono-Regular, SF Mono, Menlo, Consolas, Liberation Mono, monospace;
}

code {
  background: var(--code-bg);
  border-radius: 6px;
  font-size: 85%;
  margin: 0;
  padding: 0.2em 0.4em;
}

pre {
  background: var(--code-bg);
  border-radius: 6px;
  font-size: 85%;
  line-height: 1.45;
  overflow-x: auto;
  padding: 16px;
}

pre code {
  background: transparent;
  padding: 0;
}

table {
  border-collapse: collapse;
  display: block;
  max-width: 100%;
  overflow-x: auto;
  width: 100%;
}

th, td {
  border: 1px solid var(--border);
  padding: 6px 13px;
}

th {
  font-weight: 600;
}

tr {
  background: var(--panel);
  border-top: 1px solid var(--border);
}

tr:nth-child(2n) {
  background: var(--table-alt);
}

img {
  height: auto;
  max-width: 100%;
}

input[type="checkbox"] {
  margin: 0 0.2em 0.25em -1.4em;
  vertical-align: middle;
}

hr {
  background: var(--border-muted);
  border: 0;
  height: 0.25em;
  margin: 24px 0;
  padding: 0;
}

@media (max-width: 620px) {
  .reader {
    padding: 16px;
  }

  .markdown-body {
    border: 0;
    border-radius: 0;
    padding: 0;
  }
}
"#
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
