use crate::Result;
use katex::{KatexContext, Settings, render_to_string};
use lumis::formatter::Formatter;
use lumis::formatters::html_multi_themes::HtmlMultiThemesBuilder;
use lumis::languages::Language;
use lumis::themes;
use pulldown_cmark::{BlockQuoteKind, CodeBlockKind, Event, Options, Parser, Tag, TagEnd, html};
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
    let stats_html = render_reading_stats(&markdown);
    let script = if serve {
        format!(
            "{COPY_SCRIPT}\n{READER_SCRIPT}\n{}",
            live_reload_script(file)?
        )
    } else {
        format!("{COPY_SCRIPT}\n{READER_SCRIPT}")
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
  <link rel="icon" href="data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 32 32'><rect width='32' height='32' rx='8' fill='%232563eb'/><text x='16' y='21' font-family='monospace' font-size='15' font-weight='700' fill='white' text-anchor='middle'>md</text></svg>">
  {theme_boot}
  <style>{css}</style>
</head>
<body>
  <div class="reading-progress" aria-hidden="true"><div class="reading-progress-bar"></div></div>
  <div class="reader">
    <header class="reader-header">
      <div class="reader-header-left">
        <div class="reader-identity">
          <span class="reader-mark" aria-hidden="true">md</span>
          <span class="file-name" title="{title}">{title}</span>
        </div>
        {stats_html}
      </div>
      <div class="reader-actions">
        <button type="button" class="icon-btn toc-toggle" aria-label="Toggle table of contents" aria-expanded="false" title="Show table of contents" hidden>
          <svg class="toc-toggle-icon" width="15" height="15" viewBox="0 0 16 16" fill="none" aria-hidden="true">
            <rect x="1.75" y="2.5" width="12.5" height="11" rx="2" stroke="currentColor" stroke-width="1.4"/>
            <path d="M10.25 2.5v11" stroke="currentColor" stroke-width="1.3"/>
            <rect class="toc-icon-pane" x="10.75" y="3" width="3" height="10" rx="1" fill="currentColor"/>
          </svg>
          <span class="toc-toggle-label">Contents</span>
          <span class="toc-switch" aria-hidden="true"><span class="toc-switch-thumb"></span></span>
        </button>
        <div class="theme-switcher" role="group" aria-label="Color theme">
          <button type="button" class="theme-btn" data-theme-value="auto" title="System theme" aria-label="System theme" aria-pressed="true">
            <svg width="14" height="14" viewBox="0 0 16 16" fill="none" aria-hidden="true"><rect x="2" y="2.5" width="12" height="8.5" rx="1.5" stroke="currentColor" stroke-width="1.4"/><path d="M5.5 13.5h5M8 11v2.5" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/></svg>
          </button>
          <button type="button" class="theme-btn" data-theme-value="light" title="Light theme" aria-label="Light theme" aria-pressed="false">
            <svg width="14" height="14" viewBox="0 0 16 16" fill="none" aria-hidden="true"><circle cx="8" cy="8" r="3" stroke="currentColor" stroke-width="1.4"/><path d="M8 1.75v1.5M8 12.75v1.5M1.75 8h1.5M12.75 8h1.5M3.6 3.6l1.05 1.05M11.35 11.35l1.05 1.05M12.4 3.6l-1.05 1.05M4.65 11.35L3.6 12.4" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/></svg>
          </button>
          <button type="button" class="theme-btn" data-theme-value="sepia" title="Warm paper theme" aria-label="Warm paper theme" aria-pressed="false">
            <svg width="14" height="14" viewBox="0 0 16 16" fill="none" aria-hidden="true"><path d="M2.5 3.5c1.6-.6 3.9-.6 5.5.4 1.6-1 3.9-1 5.5-.4v9c-1.6-.6-3.9-.6-5.5.4-1.6-1-3.9-1-5.5-.4v-9Z" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round"/><path d="M8 3.9v9" stroke="currentColor" stroke-width="1.4"/></svg>
          </button>
          <button type="button" class="theme-btn" data-theme-value="dark" title="Dark theme" aria-label="Dark theme" aria-pressed="false">
            <svg width="14" height="14" viewBox="0 0 16 16" fill="none" aria-hidden="true"><path d="M13.2 9.8A5.5 5.5 0 0 1 6.2 2.8a5.5 5.5 0 1 0 7 7Z" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round"/></svg>
          </button>
        </div>
      </div>
    </header>
    <div class="reader-shell">
      <main class="reader-main">
        <article class="markdown-body">
          {body}
        </article>
      </main>
      <aside class="reader-toc" aria-label="Table of contents" hidden>
        <div class="toc-inner">
          <div class="toc-header">
            <span class="toc-title">On this page</span>
            <button type="button" class="toc-close" aria-label="Close table of contents">
              <svg width="14" height="14" viewBox="0 0 16 16" fill="none" aria-hidden="true"><path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/></svg>
            </button>
          </div>
          <nav class="toc-nav"></nav>
        </div>
      </aside>
    </div>
  </div>
  <div class="toc-backdrop" hidden></div>
  <button type="button" class="back-to-top" aria-label="Back to top" title="Back to top" hidden>
    <svg width="16" height="16" viewBox="0 0 16 16" fill="none" aria-hidden="true"><path d="M8 12.5V3.5M4 7.5L8 3.5l4 4" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg>
  </button>
  <div class="lightbox" role="dialog" aria-modal="true" aria-label="Image preview" hidden>
    <button type="button" class="lightbox-close" aria-label="Close image preview">
      <svg width="18" height="18" viewBox="0 0 16 16" fill="none" aria-hidden="true"><path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/></svg>
    </button>
    <figure class="lightbox-figure">
      <img class="lightbox-img" alt="">
      <figcaption class="lightbox-caption"></figcaption>
    </figure>
  </div>
  {script}
</body>
</html>"#,
        title = escape_html(title),
        base_href = escape_html(&base_href),
        theme_boot = THEME_BOOT_SCRIPT,
        css = reader_css(),
        stats_html = stats_html,
        body = body,
        script = script
    ))
}

/// Applies the persisted theme preference before first paint to prevent flashing.
const THEME_BOOT_SCRIPT: &str = r#"<script>(function(){try{var t=localStorage.getItem("md-reader-theme");if(t==="light"||t==="dark"||t==="sepia"){document.documentElement.setAttribute("data-theme",t);}}catch(e){}})();</script>"#;

/// Enhances code blocks with a minimal corner language label and a hover-revealed
/// copy button so code blocks stay visually clean at rest.
const COPY_SCRIPT: &str = r#"<script>
(function () {
  var LABELS = {
    js: "js", javascript: "js", ts: "ts", typescript: "ts",
    py: "python", python: "python", rs: "rust", rust: "rust", sh: "sh", bash: "bash",
    zsh: "zsh", shell: "shell", yml: "yaml", yaml: "yaml", md: "markdown", markdown: "markdown",
    cpp: "c++", cxx: "c++", c: "c", cs: "c#", go: "go", java: "java", sql: "sql",
    json: "json", html: "html", css: "css", toml: "toml", tex: "latex", text: ""
  };

  function detectLang(code) {
    var cls = code.className || "";
    var match = cls.match(/\blanguage-([a-zA-Z0-9_+#.-]+)/);
    if (!match) return "";
    var raw = match[1].toLowerCase();
    return Object.prototype.hasOwnProperty.call(LABELS, raw) ? LABELS[raw] : raw;
  }

  function enhanceCodeBlocks() {
    var pres = document.querySelectorAll(".markdown-body pre");
    for (var i = 0; i < pres.length; i++) {
      var pre = pres[i];
      if (pre.dataset.mdCopy) continue;
      var code = pre.querySelector("code");
      if (!code) continue;
      pre.dataset.mdCopy = "1";

      var wrapper = document.createElement("div");
      wrapper.className = "code-block";

      var langLabel = detectLang(code);
      if (langLabel) {
        var lang = document.createElement("span");
        lang.className = "code-lang";
        lang.textContent = langLabel;
        lang.setAttribute("aria-hidden", "true");
        wrapper.appendChild(lang);
      }

      var button = document.createElement("button");
      button.type = "button";
      button.className = "code-copy";
      button.textContent = "Copy";
      button.setAttribute("aria-label", "Copy code to clipboard");

      (function (code, btn) {
        var timer = null;
        btn.addEventListener("click", function () {
          var text = code.innerText;
          function copied() {
            btn.textContent = "Copied ✓";
            btn.classList.add("is-copied");
            if (timer) clearTimeout(timer);
            timer = setTimeout(function () {
              btn.textContent = "Copy";
              btn.classList.remove("is-copied");
            }, 1600);
          }
          if (navigator.clipboard && navigator.clipboard.writeText) {
            navigator.clipboard.writeText(text).then(copied, function () {});
          } else {
            var textarea = document.createElement("textarea");
            textarea.value = text;
            textarea.style.position = "fixed";
            textarea.style.opacity = "0";
            document.body.appendChild(textarea);
            textarea.select();
            try { document.execCommand("copy"); copied(); } catch (e) {}
            document.body.removeChild(textarea);
          }
        });
      })(code, button);

      if (pre.parentNode) {
        pre.parentNode.insertBefore(wrapper, pre);
        wrapper.appendChild(pre);
        wrapper.appendChild(button);
      }
    }
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", enhanceCodeBlocks);
  } else {
    enhanceCodeBlocks();
  }
})();
</script>"#;

/// Progressive reader UX enhancements: theme switcher, heading anchors & TOC
/// with scroll spy, responsive table wrappers, image captions & lightbox,
/// reading progress bar, and back-to-top button.
const READER_SCRIPT: &str = r##"<script>
(function () {
  function initTheme() {
    var root = document.documentElement;
    var buttons = document.querySelectorAll(".theme-btn");
    var current = root.getAttribute("data-theme") || "auto";

    function sync(value) {
      current = value;
      if (value === "auto") {
        root.removeAttribute("data-theme");
      } else {
        root.setAttribute("data-theme", value);
      }
      for (var i = 0; i < buttons.length; i++) {
        var active = buttons[i].getAttribute("data-theme-value") === value;
        buttons[i].setAttribute("aria-pressed", active ? "true" : "false");
        buttons[i].classList.toggle("is-active", active);
      }
      try {
        if (value === "auto") {
          localStorage.removeItem("md-reader-theme");
        } else {
          localStorage.setItem("md-reader-theme", value);
        }
      } catch (e) {}
    }

    sync(current);
    for (var i = 0; i < buttons.length; i++) {
      buttons[i].addEventListener("click", function () {
        sync(this.getAttribute("data-theme-value") || "auto");
      });
    }
  }

  function slugify(text) {
    var slug = text
      .trim()
      .toLowerCase()
      .replace(/[^\p{L}\p{N}\s-]/gu, "")
      .replace(/\s+/g, "-")
      .replace(/-+/g, "-")
      .replace(/^-|-$/g, "");
    return slug || "section";
  }

  function initHeadingsAndToc() {
    var article = document.querySelector(".markdown-body");
    var shell = document.querySelector(".reader-shell");
    var tocAside = document.querySelector(".reader-toc");
    var tocNav = document.querySelector(".toc-nav");
    var tocToggle = document.querySelector(".toc-toggle");
    var tocClose = document.querySelector(".toc-close");
    var backdrop = document.querySelector(".toc-backdrop");
    if (!article) return;

    var allHeadings = article.querySelectorAll("h1, h2, h3, h4");
    var headings = [];
    var usedIds = Object.create(null);

    var docBaseUrl = window.location.href.split("#")[0];

    for (var i = 0; i < allHeadings.length; i++) {
      var h = allHeadings[i];
      if (h.closest(".markdown-alert")) continue;
      var text = (h.textContent || "").trim();
      if (!text) continue;

      if (!h.id) {
        var base = slugify(text);
        var candidate = base;
        var n = 2;
        while (usedIds[candidate] || document.getElementById(candidate)) {
          candidate = base + "-" + n;
          n++;
        }
        h.id = candidate;
      }
      usedIds[h.id] = true;

      if (!h.querySelector(".heading-anchor")) {
        var anchor = document.createElement("a");
        anchor.className = "heading-anchor";
        anchor.href = docBaseUrl + "#" + encodeURIComponent(h.id);
        anchor.textContent = "#";
        anchor.setAttribute("aria-label", "Link to " + text);
        h.appendChild(anchor);
      }

      headings.push({ el: h, level: parseInt(h.tagName.slice(1), 10), id: h.id, text: text });
    }

    function scrollToId(id, pushHistory) {
      if (!id) {
        window.scrollTo({ top: 0, behavior: "smooth" });
        return true;
      }
      var target = document.getElementById(id);
      if (!target) return false;
      var prev = document.querySelector(".is-hash-target");
      if (prev) prev.classList.remove("is-hash-target");
      target.classList.add("is-hash-target");
      target.scrollIntoView({ behavior: "smooth", block: "start" });
      if (pushHistory) {
        try {
          history.pushState(null, "", docBaseUrl + "#" + encodeURIComponent(id));
        } catch (e) {}
      }
      return true;
    }

    // Intercept all in-document "#..." links (and same-document hash URLs) so
    // <base href="..."> (used for resolving relative images/files from the
    // source Markdown directory) does not hijack TOC, heading anchor, footnote,
    // or internal section clicks.
    document.addEventListener("click", function (event) {
      if (event.defaultPrevented || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) {
        return;
      }
      var link = event.target.closest("a[href]");
      if (!link) return;
      var rawHref = link.getAttribute("href") || "";
      var fragment = null;
      if (rawHref.charAt(0) === "#") {
        fragment = rawHref.slice(1);
      } else if (link.href && link.href.split("#")[0] === docBaseUrl && link.hash) {
        fragment = link.hash.slice(1);
      }
      if (fragment === null) return;
      var id = "";
      try {
        id = decodeURIComponent(fragment);
      } catch (e) {
        id = fragment;
      }
      if (scrollToId(id, true)) {
        event.preventDefault();
      }
    });

    window.addEventListener("popstate", function () {
      if (location.hash) {
        try {
          scrollToId(decodeURIComponent(location.hash.slice(1)), false);
        } catch (e) {}
      }
    });

    if (location.hash) {
      var targetId = "";
      try {
        targetId = decodeURIComponent(location.hash.slice(1));
      } catch (e) {
        targetId = location.hash.slice(1);
      }
      var target = document.getElementById(targetId);
      if (target) {
        setTimeout(function () { target.scrollIntoView(); }, 0);
      }
    }

    if (headings.length < 2 || !tocAside || !tocNav || !shell) return;

    // If there's a single top-level H1 acting as document title plus subheadings,
    // keep the TOC focused on the sections underneath unless there are multiple H1s.
    var h1Count = 0;
    for (var j = 0; j < headings.length; j++) {
      if (headings[j].level === 1) h1Count++;
    }
    var tocItems = headings;
    if (h1Count === 1 && headings[0].level === 1 && headings.length > 2) {
      tocItems = headings.slice(1);
    }

    var minLevel = 6;
    for (var k = 0; k < tocItems.length; k++) {
      if (tocItems[k].level < minLevel) minLevel = tocItems[k].level;
    }

    var list = document.createElement("ul");
    list.className = "toc-list";
    var links = [];

    for (var m = 0; m < tocItems.length; m++) {
      var item = tocItems[m];
      var li = document.createElement("li");
      li.className = "toc-item toc-depth-" + Math.min(item.level - minLevel, 3);
      var a = document.createElement("a");
      a.className = "toc-link";
      a.href = docBaseUrl + "#" + encodeURIComponent(item.id);
      a.textContent = item.text;
      a.dataset.targetId = item.id;
      li.appendChild(a);
      list.appendChild(li);
      links.push({ id: item.id, el: item.el, link: a });
    }

    function isDesktopToc() {
      return window.innerWidth >= 1240;
    }

    function syncTocToggle() {
      if (!tocToggle) return;
      var open = isDesktopToc()
        ? !shell.classList.contains("toc-collapsed")
        : document.body.classList.contains("toc-open");
      tocToggle.setAttribute("aria-expanded", open ? "true" : "false");
      tocToggle.classList.toggle("is-active", open);
      tocToggle.title = open ? "Hide table of contents" : "Show table of contents";
    }

    tocNav.appendChild(list);
    tocAside.hidden = false;
    shell.setAttribute("data-has-toc", "true");
    if (tocToggle) {
      tocToggle.hidden = false;
      syncTocToggle();
    }

    function setDrawerOpen(open) {
      document.body.classList.toggle("toc-open", open);
      if (backdrop) backdrop.hidden = !open;
      syncTocToggle();
    }

    if (tocToggle) {
      tocToggle.addEventListener("click", function () {
        if (isDesktopToc()) {
          shell.classList.toggle("toc-collapsed");
          syncTocToggle();
        } else {
          setDrawerOpen(!document.body.classList.contains("toc-open"));
        }
      });
    }
    window.addEventListener("resize", syncTocToggle, { passive: true });
    if (tocClose) {
      tocClose.addEventListener("click", function () { setDrawerOpen(false); });
    }
    if (backdrop) {
      backdrop.addEventListener("click", function () { setDrawerOpen(false); });
    }
    tocNav.addEventListener("click", function (event) {
      if (event.target.closest(".toc-link")) {
        setDrawerOpen(false);
      }
    });
    document.addEventListener("keydown", function (event) {
      if (event.key === "Escape" && document.body.classList.contains("toc-open")) {
        setDrawerOpen(false);
      }
    });

    function updateActiveToc() {
      var scrollTop = window.scrollY || document.documentElement.scrollTop;
      var activeId = links[0].id;
      for (var idx = 0; idx < links.length; idx++) {
        var top = links[idx].el.getBoundingClientRect().top + scrollTop;
        if (top <= scrollTop + 110) {
          activeId = links[idx].id;
        } else {
          break;
        }
      }
      for (var idx2 = 0; idx2 < links.length; idx2++) {
        links[idx2].link.classList.toggle("is-active", links[idx2].id === activeId);
      }
    }

    updateActiveToc();
    window.addEventListener("scroll", updateActiveToc, { passive: true });
  }

  function initTables() {
    var tables = document.querySelectorAll(".markdown-body table");
    for (var i = 0; i < tables.length; i++) {
      var table = tables[i];
      if (table.parentElement && table.parentElement.classList.contains("table-wrap")) continue;
      var wrap = document.createElement("div");
      wrap.className = "table-wrap";
      table.parentNode.insertBefore(wrap, table);
      wrap.appendChild(table);
    }
  }

  function initImagesAndLightbox() {
    var lightbox = document.querySelector(".lightbox");
    var lightboxImg = document.querySelector(".lightbox-img");
    var lightboxCaption = document.querySelector(".lightbox-caption");
    var lightboxClose = document.querySelector(".lightbox-close");
    if (!lightbox || !lightboxImg) return;

    function openLightbox(img) {
      lightboxImg.src = img.currentSrc || img.src;
      lightboxImg.alt = img.alt || "";
      var caption = img.getAttribute("title") || img.alt || "";
      if (lightboxCaption) {
        lightboxCaption.textContent = caption;
        lightboxCaption.hidden = !caption;
      }
      lightbox.hidden = false;
      document.body.classList.add("lightbox-open");
    }

    function closeLightbox() {
      lightbox.hidden = true;
      document.body.classList.remove("lightbox-open");
    }

    var imgs = document.querySelectorAll(".markdown-body img");
    for (var i = 0; i < imgs.length; i++) {
      var img = imgs[i];
      // Render subtle caption for standalone images that carry a title attribute.
      var parent = img.parentElement;
      if (parent && parent.tagName === "P" && parent.childNodes.length === 1 && img.title) {
        var fig = document.createElement("figure");
        fig.className = "md-figure";
        var cap = document.createElement("figcaption");
        cap.textContent = img.title;
        parent.parentNode.insertBefore(fig, parent);
        fig.appendChild(img);
        fig.appendChild(cap);
        parent.remove();
      }
      if (!img.closest("a")) {
        img.classList.add("zoomable");
        img.addEventListener("click", function () { openLightbox(this); });
      }
    }

    lightbox.addEventListener("click", function (event) {
      if (event.target === lightbox || event.target.closest(".lightbox-close")) {
        closeLightbox();
      }
    });
    if (lightboxClose) {
      lightboxClose.addEventListener("click", closeLightbox);
    }
    document.addEventListener("keydown", function (event) {
      if (event.key === "Escape" && !lightbox.hidden) {
        closeLightbox();
      }
    });
  }

  function initScrollChrome() {
    var bar = document.querySelector(".reading-progress-bar");
    var backBtn = document.querySelector(".back-to-top");
    var ticking = false;

    function update() {
      ticking = false;
      var scrollTop = window.scrollY || document.documentElement.scrollTop;
      var docHeight = document.documentElement.scrollHeight - window.innerHeight;
      if (bar) {
        var pct = docHeight > 0 ? Math.min(100, Math.max(0, (scrollTop / docHeight) * 100)) : 0;
        bar.style.width = pct.toFixed(2) + "%";
      }
      if (backBtn) {
        backBtn.hidden = scrollTop < 380;
      }
    }

    window.addEventListener("scroll", function () {
      if (!ticking) {
        ticking = true;
        requestAnimationFrame(update);
      }
    }, { passive: true });
    update();

    if (backBtn) {
      backBtn.addEventListener("click", function () {
        window.scrollTo({ top: 0, behavior: "smooth" });
      });
    }
  }

  function init() {
    initTheme();
    initTables();
    initHeadingsAndToc();
    initImagesAndLightbox();
    initScrollChrome();
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", init);
  } else {
    init();
  }
})();
</script>"##;

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

/// Estimates word count (counting CJK ideographs individually and Latin/numeric
/// runs as words) and reading time for the header metadata badge.
fn estimate_reading_stats(markdown: &str) -> (usize, usize) {
    let mut count = 0usize;
    let mut in_latin_word = false;

    for ch in markdown.chars() {
        if is_cjk(ch) {
            if in_latin_word {
                count += 1;
                in_latin_word = false;
            }
            count += 1;
        } else if ch.is_alphanumeric() {
            in_latin_word = true;
        } else if in_latin_word {
            count += 1;
            in_latin_word = false;
        }
    }
    if in_latin_word {
        count += 1;
    }

    let minutes = if count == 0 {
        0
    } else {
        count.div_ceil(240).max(1)
    };
    (count, minutes)
}

fn is_cjk(ch: char) -> bool {
    matches!(
        ch as u32,
        0x4E00..=0x9FFF
            | 0x3400..=0x4DBF
            | 0xF900..=0xFAFF
            | 0x3040..=0x30FF
            | 0xAC00..=0xD7AF
    )
}

fn render_reading_stats(markdown: &str) -> String {
    let (words, minutes) = estimate_reading_stats(markdown);
    if words == 0 {
        return String::new();
    }
    let word_label = if words >= 1000 {
        format!("{:.1}k words", words as f64 / 1000.0)
    } else if words == 1 {
        "1 word".to_string()
    } else {
        format!("{words} words")
    };
    format!(r#"<span class="reader-stats">{word_label} · {minutes} min read</span>"#)
}

/// Syntax-highlights a fenced code block with lumis, returning `None` when the
/// language is unknown or formatting fails so the caller can fall back to a
/// plain code block.
fn highlight_code(lang: &str, code: &str) -> Option<String> {
    let language = Language::guess(Some(lang), "");
    if language == Language::PlainText {
        return None;
    }

    // Markdown fenced code blocks include the newline before the closing fence;
    // strip that single trailing newline so lumis does not emit an extra blank
    // trailing `<div class="l-line">` row.
    let code = code
        .strip_suffix("\r\n")
        .or_else(|| code.strip_suffix('\n'))
        .unwrap_or(code);

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

fn alert_open_html(kind: BlockQuoteKind) -> &'static str {
    match kind {
        BlockQuoteKind::Note => {
            r#"<div class="markdown-alert markdown-alert-note" role="note"><p class="markdown-alert-title"><svg class="alert-icon" viewBox="0 0 16 16" width="16" height="16" fill="currentColor" aria-hidden="true"><path d="M0 8a8 8 0 1 1 16 0A8 8 0 0 1 0 8Zm8-6.5a6.5 6.5 0 1 0 0 13 6.5 6.5 0 0 0 0-13ZM6.5 7.75A.75.75 0 0 1 7.25 7h1a.75.75 0 0 1 .75.75v2.75h.25a.75.75 0 0 1 0 1.5h-2a.75.75 0 0 1 0-1.5h.25v-2h-.25a.75.75 0 0 1-.75-.75ZM8 6a1 1 0 1 1 0-2 1 1 0 0 1 0 2Z"/></svg><span>Note</span></p>"#
        }
        BlockQuoteKind::Tip => {
            r#"<div class="markdown-alert markdown-alert-tip" role="note"><p class="markdown-alert-title"><svg class="alert-icon" viewBox="0 0 16 16" width="16" height="16" fill="currentColor" aria-hidden="true"><path d="M8 1.5c-2.363 0-4 1.69-4 3.75 0 .984.424 1.625.984 2.304l.214.253c.223.264.47.556.673.848.284.411.537.896.621 1.49a.75.75 0 0 1-1.484.211c-.04-.282-.163-.547-.37-.847a8.456 8.456 0 0 0-.542-.68c-.084-.1-.173-.205-.268-.32C3.201 7.75 2.5 6.766 2.5 5.25 2.5 2.31 4.863 0 8 0s5.5 2.31 5.5 5.25c0 1.516-.701 2.5-1.328 3.259-.095.115-.184.22-.268.319-.207.245-.383.453-.541.681-.208.3-.33.565-.37.847a.751.751 0 0 1-1.485-.212c.084-.593.337-1.078.621-1.489.203-.292.45-.584.673-.848.075-.088.147-.173.213-.253.561-.679.985-1.32.985-2.304 0-2.06-1.637-3.75-4-3.75ZM5.75 12h4.5a.75.75 0 0 1 0 1.5h-4.5a.75.75 0 0 1 0-1.5ZM6 15.25a.75.75 0 0 1 .75-.75h2.5a.75.75 0 0 1 0 1.5h-2.5a.75.75 0 0 1-.75-.75Z"/></svg><span>Tip</span></p>"#
        }
        BlockQuoteKind::Important => {
            r#"<div class="markdown-alert markdown-alert-important" role="note"><p class="markdown-alert-title"><svg class="alert-icon" viewBox="0 0 16 16" width="16" height="16" fill="currentColor" aria-hidden="true"><path d="M0 1.75C0 .784.784 0 1.75 0h12.5C15.216 0 16 .784 16 1.75v9.5A1.75 1.75 0 0 1 14.25 13H8.06l-2.573 2.573A1.458 1.458 0 0 1 3 14.543V13H1.75A1.75 1.75 0 0 1 0 11.25Zm1.75-.25a.25.25 0 0 0-.25.25v9.5c0 .138.112.25.25.25h2a.75.75 0 0 1 .75.75v2.19l2.72-2.72a.749.749 0 0 1 .53-.22h6.5a.25.25 0 0 0 .25-.25v-9.5a.25.25 0 0 0-.25-.25Zm7 2.25v2.5a.75.75 0 0 1-1.5 0v-2.5a.75.75 0 0 1 1.5 0ZM9 9a1 1 0 1 1-2 0 1 1 0 0 1 2 0Z"/></svg><span>Important</span></p>"#
        }
        BlockQuoteKind::Warning => {
            r#"<div class="markdown-alert markdown-alert-warning" role="note"><p class="markdown-alert-title"><svg class="alert-icon" viewBox="0 0 16 16" width="16" height="16" fill="currentColor" aria-hidden="true"><path d="M6.457 1.047c.659-1.234 2.427-1.234 3.086 0l6.082 11.378A1.75 1.75 0 0 1 14.082 15H1.918a1.75 1.75 0 0 1-1.543-2.575Zm1.763.707a.25.25 0 0 0-.44 0L1.698 13.132a.25.25 0 0 0 .22.368h12.164a.25.25 0 0 0 .22-.368Zm.53 3.996v2.5a.75.75 0 0 1-1.5 0v-2.5a.75.75 0 0 1 1.5 0ZM9 11a1 1 0 1 1-2 0 1 1 0 0 1 2 0Z"/></svg><span>Warning</span></p>"#
        }
        BlockQuoteKind::Caution => {
            r#"<div class="markdown-alert markdown-alert-caution" role="note"><p class="markdown-alert-title"><svg class="alert-icon" viewBox="0 0 16 16" width="16" height="16" fill="currentColor" aria-hidden="true"><path d="M4.47.22A.749.749 0 0 1 5 0h6c.199 0 .389.079.53.22l4.25 4.25c.141.14.22.331.22.53v6a.749.749 0 0 1-.22.53l-4.25 4.25A.749.749 0 0 1 11 16H5a.749.749 0 0 1-.53-.22L.22 11.53A.749.749 0 0 1 0 11V5c0-.199.079-.389.22-.53Zm.84 1.28L1.5 5.31v5.38l3.81 3.81h5.38l3.81-3.81V5.31L10.69 1.5ZM8 4a.75.75 0 0 1 .75.75v3.5a.75.75 0 0 1-1.5 0v-3.5A.75.75 0 0 1 8 4Zm0 8a1 1 0 1 1 0-2 1 1 0 0 1 0 2Z"/></svg><span>Caution</span></p>"#
        }
    }
}

fn render_markdown(markdown: &str) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_HEADING_ATTRIBUTES);
    options.insert(Options::ENABLE_MATH);
    options.insert(Options::ENABLE_GFM);

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
    // original Start/Text/End events are rebuilt unchanged. Also upgrade GFM
    // alert blockquotes into styled callout containers with icon headers.
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
            Event::Start(Tag::BlockQuote(Some(kind))) => {
                events.push(Event::Html(alert_open_html(kind).into()));
            }
            Event::End(TagEnd::BlockQuote(Some(_))) => {
                events.push(Event::Html("</div>\n".into()));
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
    fn renders_gfm_alerts_with_titles_and_icons() {
        let html = render_markdown(
            "> [!NOTE]\n> Helpful context.\n\n> [!WARNING]\n> Be careful here.\n\n> Plain quote.",
        );

        assert!(html.contains(r#"class="markdown-alert markdown-alert-note""#));
        assert!(html.contains("<span>Note</span>"));
        assert!(html.contains(r#"class="markdown-alert markdown-alert-warning""#));
        assert!(html.contains("<span>Warning</span>"));
        assert!(html.contains("<blockquote>\n<p>Plain quote.</p>\n</blockquote>"));
    }

    #[test]
    fn estimates_reading_stats_for_latin_and_cjk() {
        assert_eq!(estimate_reading_stats(""), (0, 0));
        assert_eq!(estimate_reading_stats("hello world"), (2, 1));
        assert_eq!(estimate_reading_stats("你好世界 hello"), (5, 1));
        assert!(render_reading_stats("你好世界 hello").contains("5 words · 1 min read"));
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
        assert!(html.contains("data-line=\"1\""));
        assert!(!html.contains("data-line=\"2\""));
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
    fn html_includes_theme_switcher_and_toc_containers() {
        let file = temp_markdown_file("# Title\n\n## Section A\n\n## Section B\n");
        let html = render_document(&file, false).unwrap();

        assert!(html.contains("theme-switcher"));
        assert!(html.contains("reader-toc"));
        assert!(html.contains("reading-progress-bar"));
        assert!(html.contains("md-reader-theme"));
        assert!(html.contains("scrollToId"));
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
        assert!(css.contains(".markdown-alert-note"));
        assert!(css.contains(r#"html[data-theme="sepia"]"#));
        assert!(css.contains(r#"html[data-theme="dark"]"#));
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
