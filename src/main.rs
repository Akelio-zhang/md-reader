mod cli;
mod render;
mod server;

use std::{env, error::Error, fs, path::Path, process::Command};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    let flags: Vec<&String> = args.iter().skip(1).collect();

    if flags
        .iter()
        .any(|arg| matches!(arg.as_str(), "-h" | "--help"))
    {
        println!("{}", cli::usage());
        return Ok(());
    }
    if flags
        .iter()
        .any(|arg| matches!(arg.as_str(), "-V" | "--version"))
    {
        println!("md-reader {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let config = cli::Config::parse(args)?;
    if config.mode == cli::Mode::Preview {
        if config.host_set {
            eprintln!("warning: --host is only used in --serve mode; ignoring it");
        }
        if config.port_set {
            eprintln!("warning: --port is only used in --serve mode; ignoring it");
        }
    }

    let file = fs::canonicalize(&config.file)?;

    if !file.is_file() {
        return Err(format!("not a file: {}", file.display()).into());
    }

    match config.mode {
        cli::Mode::Preview => preview_file(&file, config.open_browser),
        cli::Mode::Serve => server::serve_file(&file, &config),
    }
}

fn preview_file(file: &Path, open_browser: bool) -> Result<()> {
    let html = render::render_document(file, false)?;
    let preview_file = render::write_preview_file(file, &html)?;

    println!("Rendered {}", file.display());
    println!("Preview {}", preview_file.display());

    if open_browser {
        open_in_browser(&preview_file.to_string_lossy())?;
    }

    Ok(())
}

fn open_in_browser(target: &str) -> Result<()> {
    let status = if cfg!(target_os = "macos") {
        Command::new("open").arg(target).status()?
    } else if cfg!(target_os = "windows") {
        Command::new("cmd")
            .args(["/C", "start", "", target])
            .status()?
    } else {
        Command::new("xdg-open").arg(target).status()?
    };

    if status.success() {
        Ok(())
    } else {
        Err(format!("failed to open browser for {target}").into())
    }
}

#[cfg(test)]
pub(crate) mod test_utils {
    use std::fs::FileTimes;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    static PREVIEW_COUNTER: AtomicUsize = AtomicUsize::new(0);

    pub(crate) fn temp_markdown_dir(contents: &str) -> (PathBuf, PathBuf) {
        let n = PREVIEW_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("md-reader-tests-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("notes.md");
        std::fs::write(&file, contents).unwrap();
        (dir, file)
    }

    pub(crate) fn temp_markdown_file(contents: &str) -> PathBuf {
        let n = PREVIEW_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join("md-reader-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("notes-{}-{n}.md", std::process::id()));
        std::fs::write(&path, contents).unwrap();
        path
    }

    pub(crate) fn temp_dir(prefix: &str) -> PathBuf {
        let n = PREVIEW_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("{prefix}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    pub(crate) fn backdate(path: &Path, age: Duration) {
        // Windows' SetFileTime requires a writable handle; a read-only handle
        // fails with access denied even for plain files.
        let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        let old = std::time::SystemTime::now() - age;
        file.set_times(FileTimes::new().set_modified(old)).unwrap();
    }
}
