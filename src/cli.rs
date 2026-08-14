use crate::Result;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Config {
    pub(crate) file: PathBuf,
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) open_browser: bool,
    pub(crate) mode: Mode,
    pub(crate) host_set: bool,
    pub(crate) port_set: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Preview,
    Serve,
}

impl Config {
    pub(crate) fn parse<I, S>(args: I) -> Result<Self>
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
        let mut host_set = false;
        let mut port_set = false;

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--no-open" => open_browser = false,
                "--serve" => mode = Mode::Serve,
                "--host" => {
                    host = args
                        .next()
                        .ok_or_else(|| "--host requires a value".to_string())?;
                    host_set = true;
                }
                "--port" => {
                    let value = args
                        .next()
                        .ok_or_else(|| "--port requires a value".to_string())?;
                    port = value.parse::<u16>()?;
                    port_set = true;
                }
                _ if arg.starts_with("--host=") => {
                    host = arg["--host=".len()..].to_string();
                    host_set = true;
                }
                _ if arg.starts_with("--port=") => {
                    port = arg["--port=".len()..].parse::<u16>()?;
                    port_set = true;
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
            host_set,
            port_set,
        })
    }
}

pub(crate) fn usage() -> String {
    "Usage: md-reader [OPTIONS] <file.md>\n\nOptions:\n  --serve          Serve the file over local HTTP instead of writing a temp HTML preview\n  --host <host>    Host to bind in --serve mode (default: 127.0.0.1)\n  --port <port>    Port to bind in --serve mode (default: random available port)\n  --no-open        Render or serve without opening a browser\n  -h, --help       Show this help\n  -V, --version    Show version".to_string()
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
        assert!(!config.host_set);
        assert!(!config.port_set);
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
        assert!(config.host_set);
        assert!(config.port_set);
    }
}
