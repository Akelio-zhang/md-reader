use crate::cli::Config;
use crate::render::{file_mtime_millis, render_document};
use crate::{Result, open_in_browser};
use std::{
    ffi::OsStr,
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
};

pub(crate) fn serve_file(file: &Path, config: &Config) -> Result<()> {
    let base_dir = file.parent().unwrap_or(Path::new("/")).to_path_buf();
    let listener = TcpListener::bind((config.host.as_str(), config.port))?;
    let addr = listener.local_addr()?;
    let url = format!("http://{}:{}/", display_host(&config.host), addr.port());

    println!("Reading {}", file.display());
    println!("Serving {url}");
    println!("Press Ctrl-C to stop.");

    if config.open_browser {
        open_in_browser(&url)?;
    }

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let file = file.to_path_buf();
                let base_dir = base_dir.clone();
                std::thread::spawn(move || {
                    if let Err(err) = handle_connection(stream, &file, &base_dir) {
                        eprintln!("request failed: {err}");
                    }
                });
            }
            Err(err) => eprintln!("connection failed: {err}"),
        }
    }

    Ok(())
}

fn handle_connection(mut stream: TcpStream, file: &Path, base_dir: &Path) -> Result<()> {
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

    respond(&mut stream, &handle_request(path, file, base_dir)?)
}

fn handle_request(path: &str, file: &Path, base_dir: &Path) -> Result<Response> {
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
        _ => static_response(path, base_dir),
    }
}

fn static_response(path: &str, base_dir: &Path) -> Result<Response> {
    let decoded = percent_decode_path(path);
    let candidate = base_dir.join(decoded.trim_start_matches('/'));
    Ok(serve_static_file(&candidate, base_dir)?.unwrap_or_else(not_found))
}

/// Serves a file from `base_dir` as a response, resolving symlinks and `..`
/// segments first so nothing outside the directory can be reached.
fn serve_static_file(candidate: &Path, base_dir: &Path) -> Result<Option<Response>> {
    let base_dir = fs::canonicalize(base_dir)?;
    let canonical = match fs::canonicalize(candidate) {
        Ok(path) => path,
        Err(_) => return Ok(None),
    };

    if !canonical.starts_with(&base_dir) || !canonical.is_file() {
        return Ok(None);
    }

    let body = fs::read(&canonical)?;
    Ok(Some(Response {
        status: "200 OK",
        content_type: content_type_for(&canonical),
        body,
    }))
}

fn not_found() -> Response {
    Response {
        status: "404 Not Found",
        content_type: "text/plain; charset=utf-8",
        body: b"not found".to_vec(),
    }
}

fn content_type_for(path: &Path) -> &'static str {
    match path.extension().and_then(OsStr::to_str) {
        Some("html" | "htm") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("svg") => "image/svg+xml",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("ttf") => "font/ttf",
        Some("pdf") => "application/pdf",
        Some("json") => "application/json; charset=utf-8",
        Some("md" | "txt") => "text/plain; charset=utf-8",
        Some("mp4") => "video/mp4",
        Some("mp3") => "audio/mpeg",
        _ => "application/octet-stream",
    }
}

/// Decodes the percent-encoding used in URL paths. `+` is left as-is since it
/// only means space in query strings; malformed escapes pass through literally.
fn percent_decode_path(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut position = 0;

    while position < bytes.len() {
        if bytes[position] == b'%'
            && position + 2 < bytes.len()
            && let (Some(high), Some(low)) = (
                hex_value(bytes[position + 1]),
                hex_value(bytes[position + 2]),
            )
        {
            output.push(high << 4 | low);
            position += 3;
        } else {
            output.push(bytes[position]);
            position += 1;
        }
    }

    String::from_utf8_lossy(&output).into_owned()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

struct Response {
    status: &'static str,
    content_type: &'static str,
    body: Vec<u8>,
}

fn respond<W: Write>(stream: &mut W, response: &Response) -> Result<()> {
    write!(
        stream,
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        response.status,
        response.content_type,
        response.body.len()
    )?;
    stream.write_all(&response.body)?;
    stream.flush()?;
    Ok(())
}

/// Wildcard binds listen on every interface but are not usable as a browser
/// address, so print the loopback address instead.
fn display_host(host: &str) -> String {
    if host == "0.0.0.0" || host == "::" {
        "127.0.0.1".to_string()
    } else {
        host.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::file_mtime_millis;
    use crate::test_utils::*;

    #[test]
    fn refresh_route_reports_file_mtime_in_milliseconds() {
        let (dir, file) = temp_markdown_dir("hello");
        let response = handle_request("/refresh", &file, &dir).unwrap();

        assert_eq!(response.status, "200 OK");
        assert_eq!(response.content_type, "text/plain; charset=utf-8");
        assert_eq!(
            String::from_utf8(response.body).unwrap(),
            file_mtime_millis(&file).unwrap().to_string()
        );
    }

    #[test]
    fn serve_root_route_returns_rendered_document() {
        let (dir, file) = temp_markdown_dir("# Hello");
        let response = handle_request("/", &file, &dir).unwrap();

        assert_eq!(response.status, "200 OK");
        assert_eq!(response.content_type, "text/html; charset=utf-8");
        assert!(
            String::from_utf8(response.body)
                .unwrap()
                .contains("<h1>Hello</h1>")
        );
    }

    #[test]
    fn serve_mode_serves_files_from_the_markdown_directory() {
        let (dir, file) = temp_markdown_dir("## Hi");
        let image = b"\x89PNG\r\n\x1a\nfake png bytes";
        fs::write(dir.join("logo.png"), image).unwrap();

        let response = handle_request("/logo.png", &file, &dir).unwrap();

        assert_eq!(response.status, "200 OK");
        assert_eq!(response.content_type, "image/png");
        assert_eq!(response.body, image);
    }

    #[test]
    fn serve_mode_sets_mime_type_from_file_extension() {
        let (dir, file) = temp_markdown_dir("## Hi");
        fs::write(dir.join("style.css"), "body { margin: 0 }").unwrap();

        let response = handle_request("/style.css", &file, &dir).unwrap();

        assert_eq!(response.status, "200 OK");
        assert_eq!(response.content_type, "text/css; charset=utf-8");
    }

    #[test]
    fn serve_mode_returns_404_for_missing_static_files() {
        let (dir, file) = temp_markdown_dir("## Hi");
        let response = handle_request("/nope.png", &file, &dir).unwrap();

        assert_eq!(response.status, "404 Not Found");
    }

    #[test]
    fn serve_mode_rejects_percent_encoded_traversal() {
        let (dir, file) = temp_markdown_dir("## Hi");
        let outside =
            std::env::temp_dir().join(format!("md-reader-outside-{}.txt", std::process::id()));
        fs::write(&outside, "secret").unwrap();

        let escaped = format!(
            "/{}/md-reader-outside-{}.txt",
            "%2e%2e/".repeat(12),
            std::process::id()
        );
        let response = handle_request(&escaped, &file, &dir).unwrap();

        assert_eq!(response.status, "404 Not Found");
    }

    #[cfg(unix)]
    #[test]
    fn serve_mode_rejects_symlinks_escaping_the_directory() {
        use std::os::unix::fs::symlink;

        let (dir, file) = temp_markdown_dir("## Hi");
        let outside = std::env::temp_dir().join(format!(
            "md-reader-outside-symlink-{}.txt",
            std::process::id()
        ));
        fs::write(&outside, "secret").unwrap();
        symlink(&outside, dir.join("escape.png")).unwrap();

        let response = handle_request("/escape.png", &file, &dir).unwrap();

        assert_eq!(response.status, "404 Not Found");
    }

    #[test]
    fn responses_are_marked_no_store() {
        let response = Response {
            status: "200 OK",
            content_type: "text/plain; charset=utf-8",
            body: b"ok".to_vec(),
        };
        let mut buffer = Vec::new();

        respond(&mut buffer, &response).unwrap();

        let head = String::from_utf8(buffer).unwrap();
        assert!(head.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(head.contains("Cache-Control: no-store\r\n"));
        assert!(head.ends_with("\r\n\r\nok"));
    }

    #[test]
    fn display_host_maps_wildcard_binds_to_loopback() {
        assert_eq!(display_host("0.0.0.0"), "127.0.0.1");
        assert_eq!(display_host("::"), "127.0.0.1");
        assert_eq!(display_host("localhost"), "localhost");
        assert_eq!(display_host("192.168.1.5"), "192.168.1.5");
    }
}
