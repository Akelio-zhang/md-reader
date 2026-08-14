# md-reader

Open a Markdown file in your browser with a GitHub-like preview style.

`md-reader` is a small Rust CLI for reading local Markdown files without
starting an editor or copying content into another app. By default it renders a
temporary HTML file, opens it in your browser, and exits.

## Features

- GitHub-like Markdown preview style with light and dark color schemes
- Tables, task lists, footnotes, strikethrough, and heading attributes
- Offline LaTeX math rendering with KaTeX (`$...$`, `$$...$$`, `\(...\)`, and `\[...\]`)
- Syntax-highlighted fenced code blocks (15 common languages) with light and dark themes, and one-click code copy
- Relative images and links resolved from the Markdown file's directory
- One-shot preview mode for quick reading
- Optional local server mode with automatic refresh for editing workflows
- Single native binary for Linux, Windows, and macOS

## Install

Download a prebuilt binary from this repository's GitHub Releases page:

| Platform | Asset |
| --- | --- |
| Linux x86_64 | `md-reader-linux-x86_64.tar.gz` |
| Windows x86_64 | `md-reader-windows-x86_64.zip` |
| macOS Apple Silicon | `md-reader-macos-aarch64.tar.gz` |

Or install from source:

```sh
cargo install --path .
```

## Usage

Preview a file:

```sh
md-reader README.md
```

Render without opening the browser:

```sh
md-reader --no-open README.md
```

Serve the file over local HTTP:

```sh
md-reader --serve README.md
```

In `--serve` mode, the page reloads automatically whenever the file changes on disk.

## Options

```text
Usage: md-reader [OPTIONS] <file.md>

Options:
  --serve          Serve the file over local HTTP instead of writing a temp HTML preview
  --host <host>    Host to bind in --serve mode (default: 127.0.0.1)
  --port <port>    Port to bind in --serve mode (default: random available port)
  --no-open        Render or serve without opening a browser
  -h, --help       Show this help
  -V, --version    Show version
```

## Development

```sh
cargo test
cargo run -- README.md
```

## Releases

This repository includes a GitHub Actions workflow that:

- runs tests on push and pull requests
- builds release artifacts for Linux, Windows, and macOS
- uploads compiled binaries when a version tag is pushed

To publish version `0.1.0`:

```sh
git tag v0.1.0
git push origin v0.1.0
```

## License

MIT
