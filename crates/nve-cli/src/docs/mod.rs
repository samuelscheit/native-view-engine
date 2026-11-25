//! `cargo nve docs` subcommand (`check`, `build`, `dev`).

pub(crate) mod build;
pub(crate) mod check;
pub(crate) mod dev;

use std::path::PathBuf;

use clap::Subcommand;

#[derive(Subcommand, Debug, Clone)]
pub(crate) enum DocsAction {
    /// Validate frontmatter, internal links, heading anchors, _meta.toml manifests, and LiveDemo IDs.
    Check {
        /// Path to the documentation content directory.
        #[arg(long, default_value = "docs/content")]
        content: PathBuf,
    },
    /// Build static semantic HTML5 + Option 5 CSS pages, search.nvs1, sitemap.xml, and WASM hydration bundle.
    Build {
        /// Path to the documentation content directory.
        #[arg(long, default_value = "docs/content")]
        content: PathBuf,
        /// Output directory for the generated static site.
        #[arg(long, default_value = "dist/docs")]
        out: PathBuf,
        /// Optional deployment base path (e.g. `""` or `"/native-view-engine"`).
        #[arg(long, default_value = "")]
        base_path: String,
        /// Canonical public site origin + base URL used in sitemap.xml and OpenGraph tags.
        #[arg(
            long,
            default_value = "https://samuelscheit.github.io/native-view-engine"
        )]
        site_url: String,
        /// Skip compiling the wasm32 progressive hydration bundle (emit static SSR HTML/CSS only).
        #[arg(long, default_value_t = false)]
        skip_wasm: bool,
    },
    /// Build once, serve over HTTP, watch docs/content/, and incrementally rebuild on change.
    Dev {
        /// Path to the documentation content directory.
        #[arg(long, default_value = "docs/content")]
        content: PathBuf,
        /// Local HTTP port to serve the docs site on.
        #[arg(long, default_value_t = 4173)]
        port: u16,
        /// Skip compiling the wasm32 progressive hydration bundle.
        #[arg(long, default_value_t = false)]
        skip_wasm: bool,
    },
}

pub(crate) fn cmd_docs(action: DocsAction) {
    let result = match action {
        DocsAction::Check { content } => check::run_check(&content),
        DocsAction::Build {
            content,
            out,
            base_path,
            site_url,
            skip_wasm,
        } => build::run_build(&build::BuildOptions {
            content,
            out,
            base_path,
            site_url,
            skip_wasm,
        })
        .map(|_| ()),
        DocsAction::Dev {
            content,
            port,
            skip_wasm,
        } => dev::run_dev(&content, port, skip_wasm),
    };

    if let Err(err) = result {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}
