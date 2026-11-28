# Changelog

All notable changes to `md-reader` are documented in this file.

## [0.2.0] - 2026-11-28

### Added

- **Theme Switcher**: Switch between Auto (system), Light, Sepia (warm paper), and Dark themes with `localStorage` persistence and zero-flash boot script, synced with `lumis` syntax highlighting.
- **GFM Alerts**: Render GitHub-style alert callouts (`[!NOTE]`, `[!TIP]`, `[!IMPORTANT]`, `[!WARNING]`, `[!CAUTION]`) with semantic colors and icons.
- **Table of Contents & Anchors**: Automatic Unicode-safe heading IDs, hover `#` anchor links, sticky right-rail TOC with scroll spy on wide viewports, slide-over drawer on compact screens, and a clear toggle switch in the header.
- **Reading Chrome**: Header word count and estimated reading time (supporting both CJK and Latin text), top reading progress bar, back-to-top button, and click-to-zoom image lightbox with standalone figure captions.
- **Semantic Element Styling**: Dedicated styles for footnotes (grid alignment and target highlight), `<kbd>`, `<mark>`, `<details>/<summary>`, muted completed task items, and responsive horizontally scrollable table wrappers.

### Changed

- **Typography & Layout**: Unified reading column measure, refined CJK + Latin font stacks, improved spacing, and cleaner print styles.
- **Code Blocks**: Unified light/sepia/dark code block styling for both syntax-highlighted and plain blocks, with a subtle corner language label and hover-revealed frosted copy button.

### Fixed

- Intercept in-document `#...` links so `<base href>` in preview mode no longer hijacks TOC, heading anchor, footnote, or internal section navigation.
- Strip the single trailing newline before `lumis` highlighting to eliminate the extra blank line at the bottom of syntax-highlighted code blocks.

## [0.1.3] - 2026-10-08

### Fixed

- Fix overlapping checklist bullets and checkboxes.
- Improve checkbox sizing, first-line alignment, and spacing between tasks.
- Keep wrapped text and nested lists aligned, including tasks with multiple paragraphs.
- Use clear checked states in light and dark themes, with a native checkbox fallback in forced-colors mode.

## [0.1.2] - 2026-08-19

### Fixed

- Exclude the copy button label when copying code block contents.

### Changed

- Update GitHub Actions `upload-artifact` to v6 for the Node 24 runtime.

## [0.1.1] - 2026-08-14

### Fixed

- Fix `backdate` test helper permissions on Windows.

### Changed

- Upgrade GitHub Actions workflows to Node 24 runtimes and restrict release builds to version tags.

## [0.1.0] - 2026-08-14

### Added

- Offline LaTeX math rendering via embedded KaTeX and WOFF2 fonts (`$...$`, `$$...$$`, `\(...\)`, `\[...\]`).
- Syntax-highlighted fenced code blocks (15 common languages) via `lumis` with light/dark theme support and copy buttons.
- Local `--serve` HTTP mode with static asset serving, `HEAD` support, and automatic live reload on file changes.
- Automatic cleanup of stale temporary preview files.

### Changed

- Split codebase into `cli`, `server`, and `render` modules.
- Shrink release binaries with thin LTO and symbol stripping.

## [0.0.1] - 2026-05-23

### Added

- Initial `md-reader` CLI with one-shot HTML preview and MIT license.
