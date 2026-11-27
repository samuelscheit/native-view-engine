//! `/examples/<slug>/` portal page: the Portal Bar (brand, back link, app switcher, reset, live
//! status) around the embedded web-shell iframe, or the bundle-less fallback card.
//!
//! The chrome follows the OS like the docs site and the embedded apps: its colors are CSS custom
//! properties generated from the `nve_ui::Theme` docs presets for all four polarities (dark
//! base plus `prefers-color-scheme` / `prefers-contrast` overrides), never hand-picked literals.

use nve_docs_site::GITHUB_REPO_URL;
use nve_ui::{tab_colors, AppBrand, ColorHex, Polarity, Theme};

use super::examples::{join_base, ExampleAppSpec};

/// Portal color custom properties for one polarity.
fn polarity_vars(polarity: Polarity) -> String {
    let t = Theme::preset(polarity, AppBrand::Docs);
    let tab = tab_colors(&t, true, false);
    format!(
        "--bg:{};--bar:{};--sunken:{};--hover:{};--border:{};--text:{};--muted:{};--link:{};\
         --ok:{};--tab-bg:{};--tab-fg:{};--tab-border:{};--solid:{};--on-solid:{};",
        t.surface.canvas.hex(),
        t.surface.base.hex(),
        t.surface.sunken.hex(),
        t.surface.subtle.hex(),
        t.border.default.hex(),
        t.text.primary.hex(),
        t.text.secondary.hex(),
        t.text.link.hex(),
        t.status.success.text.hex(),
        tab.bg.hex(),
        tab.fg.hex(),
        tab.border.hex(),
        t.status.accent.solid.hex(),
        t.status.accent.on_solid.hex(),
    )
}

/// `:root` custom properties: Dark first (also the no-preference default), then OS overrides.
pub(super) fn portal_theme_css() -> String {
    format!(
        ":root{{color-scheme:dark;{dark}}}\
         @media (prefers-color-scheme: light){{:root{{color-scheme:light;{light}}}}}\
         @media (prefers-color-scheme: dark) and (prefers-contrast: more){{:root{{{hc_dark}}}}}\
         @media (prefers-color-scheme: light) and (prefers-contrast: more){{:root{{{hc_light}}}}}",
        dark = polarity_vars(Polarity::Dark),
        light = polarity_vars(Polarity::Light),
        hc_dark = polarity_vars(Polarity::HighContrastDark),
        hc_light = polarity_vars(Polarity::HighContrastLight),
    )
}

pub(super) fn render_portal_wrapper_html(
    spec: ExampleAppSpec,
    base_path: &str,
    iframe_src: Option<&str>,
    bundle_mode: &str,
) -> String {
    let home_href = join_base(base_path, "");
    let examples_href = join_base(base_path, "examples/");
    let docs_href = join_base(base_path, "docs/");
    let studio_href = join_base(base_path, "examples/studio/");
    let kitchen_sink_href = join_base(base_path, "examples/kitchen-sink/");

    let studio_active = if spec.slug == "studio" { "true" } else { "false" };
    let kitchen_sink_active = if spec.slug == "kitchen-sink" { "true" } else { "false" };
    let initial_status = if iframe_src.is_some() {
        "Booting WASM…"
    } else {
        "WASM Not Built"
    };
    let theme_css = portal_theme_css();
    let dark_canvas = Theme::preset(Polarity::Dark, AppBrand::Docs).surface.canvas.hex();
    let light_canvas = Theme::preset(Polarity::Light, AppBrand::Docs).surface.canvas.hex();

    let main_body = match iframe_src {
        Some(src) => format!(
            r#"<iframe id="nve-example-frame" class="portal-frame" title="{title}" src="{src}" data-bundle-mode="{bundle_mode}"></iframe>"#,
            title = spec.title,
        ),
        None => format!(
            r#"<div class="fallback-wrap" data-nve-example-fallback="{slug}">
  <div class="fallback-card">
    <div class="fallback-pill">Lazy WASM Bundle Not Staged</div>
    <h1>{title}</h1>
    <p>{desc}</p>
    <p class="fallback-note">This static site was built with <code>--skip-wasm</code> (or without a prebuilt <code>hosts/web-shell</code> bundle). Build and include the live WebAssembly apps locally with:</p>
    <pre><code>./hosts/web-shell/build.sh
cargo run --locked -p nve-cli -- docs build --web-shell-dist hosts/web-shell</code></pre>
    <div class="fallback-actions">
      <a class="btn-primary" href="{examples_href}">← Back to Examples</a>
      <a class="btn-secondary" href="{docs_href}">Read Documentation</a>
    </div>
  </div>
</div>"#,
            slug = spec.slug,
            title = spec.title,
            desc = spec.description,
        ),
    };

    format!(
        r##"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <meta name="theme-color" media="(prefers-color-scheme: dark)" content="{dark_canvas}" />
  <meta name="theme-color" media="(prefers-color-scheme: light)" content="{light_canvas}" />
  <title>{title} — Native View Engine</title>
  <style>
    {theme_css}
    *, *::before, *::after {{ box-sizing: border-box; }}
    html, body {{
      margin: 0; padding: 0; width: 100%; height: 100%;
      overflow: hidden;
      background: var(--bg); color: var(--text);
      font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
      -webkit-font-smoothing: antialiased;
    }}
    .portal-shell {{ display: flex; flex-direction: column; width: 100%; height: 100vh; }}
    .portal-bar {{ height: 48px; flex-shrink: 0; width: 100%; display: flex; align-items: center; justify-content: space-between; gap: 10px; padding: 0 12px; background: var(--bar); border-bottom: 1px solid var(--border); font-size: 13px; }}
    .portal-left, .portal-right {{ display: flex; align-items: center; gap: 8px; min-width: 0; }}
    .brand-pill {{ display: inline-flex; align-items: center; text-decoration: none; }}
    .brand-mark {{ padding: 2px 7px; border-radius: 6px; background: var(--tab-bg); border: 1px solid var(--tab-border); color: var(--tab-fg); font-size: 11px; font-weight: 700; letter-spacing: 0.02em; }}
    .nav-link {{ padding: 5px 8px; border-radius: 7px; color: var(--muted); text-decoration: none; font-weight: 500; white-space: nowrap; border: 1px solid transparent; }}
    .nav-link:hover {{ color: var(--text); background: var(--hover); border-color: var(--border); }}
    .switcher {{ display: inline-flex; align-items: center; gap: 3px; padding: 3px; border-radius: 9px; background: var(--sunken); border: 1px solid var(--border); }}
    .switch-tab {{ padding: 4px 10px; border-radius: 6px; color: var(--muted); text-decoration: none; font-size: 12px; font-weight: 600; white-space: nowrap; border: 1px solid transparent; }}
    .switch-tab:hover {{ color: var(--text); }}
    .switch-tab[data-active="true"] {{ background: var(--tab-bg); color: var(--tab-fg); border-color: var(--tab-border); }}
    .status-pill {{ padding: 4px 10px; border-radius: 999px; background: var(--hover); border: 1px solid var(--border); color: var(--muted); font-size: 11px; white-space: nowrap; font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace; }}
    .status-pill[data-ready="true"] {{ color: var(--ok); border-color: var(--ok); }}
    .portal-main {{ flex: 1 1 0%; width: 100%; min-height: 0; display: flex; background: var(--bg); }}
    .portal-frame {{ width: 100%; height: 100%; border: 0; display: block; background: var(--bg); }}
    .fallback-wrap {{ flex: 1; display: flex; align-items: center; justify-content: center; padding: 24px; }}
    .fallback-card {{ max-width: 620px; width: 100%; padding: 28px; border-radius: 14px; background: var(--bar); border: 1px solid var(--border); display: flex; flex-direction: column; gap: 14px; }}
    .fallback-pill {{ align-self: flex-start; padding: 3px 10px; border-radius: 999px; background: var(--tab-bg); border: 1px solid var(--tab-border); color: var(--tab-fg); font-size: 11px; font-weight: 700; }}
    .fallback-card h1 {{ margin: 0; font-size: 24px; color: var(--text); }}
    .fallback-card p {{ margin: 0; font-size: 14px; color: var(--muted); line-height: 1.55; }}
    .fallback-card pre {{ margin: 0; padding: 14px; border-radius: 10px; background: var(--sunken); border: 1px solid var(--border); overflow-x: auto; font-size: 12px; color: var(--link); font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace; }}
    .fallback-actions {{ display: flex; gap: 10px; flex-wrap: wrap; padding-top: 6px; }}
    .btn-primary {{ padding: 9px 16px; border-radius: 8px; text-decoration: none; background: var(--solid); color: var(--on-solid); font-size: 13px; font-weight: 700; }}
    .btn-secondary {{ padding: 9px 16px; border-radius: 8px; text-decoration: none; background: var(--hover); border: 1px solid var(--border); color: var(--text); font-size: 13px; font-weight: 600; }}
    @media (max-width: 720px) {{ .hide-sm {{ display: none !important; }} }}
  </style>
</head>
<body>
  <div class="portal-shell" data-nve-example="{slug}" data-bundle-mode="{bundle_mode}">
    <header class="portal-bar" role="banner">
      <div class="portal-left">
        <a class="brand-pill" href="{home_href}" aria-label="Native View Engine home">
          <span class="brand-mark">NVE</span>
        </a>
        <a class="nav-link" data-testid="portal-back-examples" href="{examples_href}" aria-label="Back to Examples">←<span class="hide-sm"> Back to Examples</span></a>
        <a class="nav-link hide-sm" data-testid="portal-docs-link" href="{docs_href}">Docs</a>
      </div>
      <div class="portal-right">
        <nav class="switcher" aria-label="Switch live example">
          <a class="switch-tab" data-testid="portal-switch-studio" href="{studio_href}" data-active="{studio_active}">Studio</a>
          <a class="switch-tab" data-testid="portal-switch-kitchen-sink" href="{kitchen_sink_href}" data-active="{kitchen_sink_active}">Kitchen Sink</a>
        </nav>
        <button id="portal-reset" class="nav-link" data-testid="portal-reset-btn" type="button" style="background:transparent;cursor:pointer;font-family:inherit;font-size:12px;" title="Clear cached snapshot and reset workspace">Reset</button>
        <span id="portal-status" class="status-pill hide-sm" data-ready="false">{initial_status}</span>
        <a class="nav-link hide-sm" href="{github_url}" target="_blank" rel="noopener noreferrer">GitHub ↗</a>
      </div>
    </header>
    <main class="portal-main">
      {main_body}
    </main>
  </div>
  <script>
    (() => {{
      const frame = document.getElementById("nve-example-frame");
      const pill = document.getElementById("portal-status");
      const resetBtn = document.getElementById("portal-reset");
      if (!frame || !pill) return;
      const baseSrc = frame.getAttribute("src") || "";
      if (new URLSearchParams(location.search).get("reset") === "1" && baseSrc && !baseSrc.includes("reset=1")) {{
        frame.src = baseSrc + (baseSrc.includes("?") ? "&" : "?") + "reset=1";
      }}
      if (resetBtn) {{
        resetBtn.addEventListener("click", () => {{
          if (!baseSrc) return;
          pill.textContent = "Resetting…";
          pill.dataset.ready = "false";
          frame.src = baseSrc + (baseSrc.includes("?") ? "&" : "?") + "reset=1";
        }});
      }}
      const poll = () => {{
        try {{
          const doc = frame.contentDocument;
          if (!doc) return;
          const app = doc.getElementById("app");
          if (app && app.dataset.nveLive === "1") {{
            pill.textContent = "WASM Live ({bundle_mode})";
            pill.dataset.ready = "true";
            return;
          }}
          if (app && app.dataset.nveRestored === "1") {{
            pill.textContent = "Instant Snapshot · Booting…";
          }} else if (app && app.querySelector("[data-nve-id]")) {{
            pill.textContent = "Instant SSR · Booting…";
          }}
        }} catch {{}}
        setTimeout(poll, 150);
      }};
      frame.addEventListener("load", poll);
      poll();
    }})();
  </script>
</body>
</html>
"##,
        title = spec.title,
        slug = spec.slug,
        github_url = GITHUB_REPO_URL,
    )
}

#[cfg(test)]
mod tests {
    use super::{polarity_vars, portal_theme_css};
    use nve_ui::Polarity;

    #[test]
    fn portal_chrome_defines_every_polarity_from_theme_tokens() {
        let css = portal_theme_css();
        let polarities = [Polarity::Dark, Polarity::Light, Polarity::HighContrastDark, Polarity::HighContrastLight];
        for p in polarities {
            assert!(css.contains(&polarity_vars(p)), "{p:?} block missing");
        }
        let blocks: std::collections::HashSet<String> = polarities.into_iter().map(polarity_vars).collect();
        assert_eq!(blocks.len(), 4, "every polarity resolves distinct chrome colors");
        assert!(css.starts_with(":root{color-scheme:dark;"), "dark is the no-preference base");
        assert!(css.contains("@media (prefers-color-scheme: light) and (prefers-contrast: more)"));
    }
}
