//! `SiteSource`, deterministic `NavTree`, and flattened `PrevNext` pagination builder.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io;
use std::path::Path;

use crate::ast::CompiledPage;
pub use crate::meta_toml::{MetaEntry, parse_meta_toml};

/// In-memory or filesystem-backed collection of `.md`, `.mdx`, and `_meta.toml` files.
#[derive(Debug, Clone, Default)]
pub struct SiteSource {
    /// Relative POSIX path (e.g. `"index.mdx"`, `"guides/_meta.toml"`) -> UTF-8 source.
    pub files: BTreeMap<String, String>,
}

impl SiteSource {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a file into the in-memory site source (normalizing path separators to `/`).
    pub fn insert(&mut self, path: impl Into<String>, content: impl Into<String>) -> &mut Self {
        let norm = normalize_rel_path(&path.into());
        self.files.insert(norm, content.into());
        self
    }

    /// Recursively load `.md`, `.mdx`, and `_meta.toml` files from a directory on disk.
    pub fn from_dir(root: &Path) -> io::Result<Self> {
        let mut source = Self::new();
        walk_dir(root, root, &mut source)?;
        Ok(source)
    }

    /// Iterate over all Markdown/MDX document entries `(relative_path, content)`.
    pub fn doc_files(&self) -> impl Iterator<Item = (&str, &str)> {
        self.files
            .iter()
            .filter(|(p, _)| p.ends_with(".md") || p.ends_with(".mdx"))
            .map(|(p, c)| (p.as_str(), c.as_str()))
    }

    /// Look up the `_meta.toml` for a folder (`""` for root, `"guides"` for `guides/_meta.toml`).
    pub fn meta_for_dir(&self, dir: &str) -> Option<&str> {
        let key = if dir.is_empty() {
            "_meta.toml".to_string()
        } else {
            format!("{dir}/_meta.toml")
        };
        self.files.get(&key).map(String::as_str)
    }
}

fn walk_dir(root: &Path, current: &Path, out: &mut SiteSource) -> io::Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(current)?.collect::<io::Result<Vec<_>>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with('.') {
            continue;
        }
        let ft = entry.file_type()?;
        if ft.is_dir() {
            walk_dir(root, &path, out)?;
        } else if ft.is_file()
            && (name_str == "_meta.toml" || name_str.ends_with(".md") || name_str.ends_with(".mdx"))
        {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            let content = std::fs::read_to_string(&path)?;
            out.insert(rel, content);
        }
    }
    Ok(())
}

pub(crate) fn normalize_rel_path(path: &str) -> String {
    path.replace('\\', "/")
        .trim_start_matches("./")
        .trim_start_matches('/')
        .to_string()
}

/// Convert a relative `.md`/`.mdx` path into a canonical site route.
pub fn route_from_source_path(path: &str) -> String {
    let norm = normalize_rel_path(path);
    let without_ext = norm
        .strip_suffix(".mdx")
        .or_else(|| norm.strip_suffix(".md"))
        .unwrap_or(&norm);
    if without_ext == "index" || without_ext.is_empty() {
        return "/".to_string();
    }
    if let Some(dir) = without_ext.strip_suffix("/index") {
        if dir.is_empty() {
            return "/".to_string();
        }
        return format!("/{dir}");
    }
    format!("/{without_ext}")
}

/// Deterministic sidebar navigation tree.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NavTree {
    pub items: Vec<NavNode>,
}

/// Node in a [`NavTree`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavNode {
    Page {
        title: String,
        route: String,
        badge: Option<String>,
        icon: Option<String>,
    },
    Folder {
        title: String,
        slug: String,
        index_route: Option<String>,
        collapsed: bool,
        badge: Option<String>,
        icon: Option<String>,
        children: Vec<NavNode>,
    },
    Separator {
        title: Option<String>,
    },
    ExternalLink {
        title: String,
        href: String,
        badge: Option<String>,
        icon: Option<String>,
    },
}

/// Single link in a flattened pagination trail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavLink {
    pub title: String,
    pub route: String,
}

/// Previous / next page links for a given route.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PrevNext {
    pub prev: Option<NavLink>,
    pub next: Option<NavLink>,
}

/// Build a deterministic [`NavTree`] and flattened [`PrevNext`] pagination map.
pub fn build_navigation(
    source: &SiteSource,
    pages: &[CompiledPage],
) -> (NavTree, BTreeMap<String, PrevNext>) {
    let mut page_by_path: HashMap<&str, &CompiledPage> = HashMap::new();
    for p in pages {
        page_by_path.insert(p.source_path.as_str(), p);
    }

    let items = build_dir_nodes("", source, &page_by_path);
    let nav = NavTree { items };

    let mut flat: Vec<NavLink> = Vec::new();
    flatten_nav_links(&nav.items, &mut flat);

    let mut prev_next = BTreeMap::new();
    for (idx, link) in flat.iter().enumerate() {
        let prev = idx.checked_sub(1).and_then(|i| flat.get(i)).cloned();
        let next = flat.get(idx + 1).cloned();
        prev_next.insert(link.route.clone(), PrevNext { prev, next });
    }

    (nav, prev_next)
}

fn build_dir_nodes(
    dir: &str,
    source: &SiteSource,
    page_by_path: &HashMap<&str, &CompiledPage>,
) -> Vec<NavNode> {
    let mut direct_pages: BTreeMap<String, &CompiledPage> = BTreeMap::new();
    let mut child_dirs: BTreeSet<String> = BTreeSet::new();
    let prefix = if dir.is_empty() {
        String::new()
    } else {
        format!("{dir}/")
    };

    for (&path, &page) in page_by_path {
        let Some(rel) = path.strip_prefix(&prefix) else {
            continue;
        };
        if let Some((sub, _)) = rel.split_once('/') {
            child_dirs.insert(sub.to_string());
        } else {
            let stem = rel
                .strip_suffix(".mdx")
                .or_else(|| rel.strip_suffix(".md"))
                .unwrap_or(rel);
            direct_pages.insert(stem.to_string(), page);
        }
    }

    let meta_entries = source
        .meta_for_dir(dir)
        .map(parse_meta_toml)
        .unwrap_or_default();

    let mut nodes = Vec::new();
    let mut consumed_pages: BTreeSet<String> = BTreeSet::new();
    let mut consumed_dirs: BTreeSet<String> = BTreeSet::new();

    for entry in &meta_entries {
        if entry.hidden {
            consumed_pages.insert(entry.key.clone());
            consumed_dirs.insert(entry.key.clone());
            continue;
        }
        if entry.separator {
            nodes.push(NavNode::Separator {
                title: entry.title.clone(),
            });
            continue;
        }
        if let Some(href) = &entry.href {
            nodes.push(NavNode::ExternalLink {
                title: entry
                    .title
                    .clone()
                    .unwrap_or_else(|| titleize_slug(&entry.key)),
                href: href.clone(),
                badge: entry.badge.clone(),
                icon: entry.icon.clone(),
            });
            continue;
        }
        if child_dirs.contains(&entry.key) {
            consumed_dirs.insert(entry.key.clone());
            let sub_dir = if dir.is_empty() {
                entry.key.clone()
            } else {
                format!("{dir}/{}", entry.key)
            };
            nodes.push(build_folder_node(
                &entry.key,
                &sub_dir,
                Some(entry),
                source,
                page_by_path,
            ));
        } else if let Some(&page) = direct_pages.get(&entry.key) {
            consumed_pages.insert(entry.key.clone());
            nodes.push(NavNode::Page {
                title: entry.title.clone().unwrap_or_else(|| page.title.clone()),
                route: page.route.clone(),
                badge: entry
                    .badge
                    .clone()
                    .or_else(|| page.frontmatter.badge.clone()),
                icon: entry.icon.clone().or_else(|| page.frontmatter.icon.clone()),
            });
        }
    }

    let mut leftovers: Vec<(i64, String, NavNode)> = Vec::new();
    for (stem, &page) in &direct_pages {
        if consumed_pages.contains(stem) || (stem == "index" && !dir.is_empty()) {
            continue;
        }
        let order = page.frontmatter.order.unwrap_or(i64::MAX);
        leftovers.push((
            order,
            stem.clone(),
            NavNode::Page {
                title: page.title.clone(),
                route: page.route.clone(),
                badge: page.frontmatter.badge.clone(),
                icon: page.frontmatter.icon.clone(),
            },
        ));
    }

    for sub in &child_dirs {
        if consumed_dirs.contains(sub) {
            continue;
        }
        let sub_dir = if dir.is_empty() {
            sub.clone()
        } else {
            format!("{dir}/{sub}")
        };
        let index_order = find_dir_index(&sub_dir, page_by_path)
            .and_then(|p| p.frontmatter.order)
            .unwrap_or(i64::MAX);
        let folder = build_folder_node(sub, &sub_dir, None, source, page_by_path);
        leftovers.push((index_order, sub.clone(), folder));
    }

    leftovers.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    for (_, _, node) in leftovers {
        nodes.push(node);
    }

    nodes
}

fn find_dir_index<'a>(
    dir: &str,
    page_by_path: &HashMap<&str, &'a CompiledPage>,
) -> Option<&'a CompiledPage> {
    let mdx = format!("{dir}/index.mdx");
    let md = format!("{dir}/index.md");
    page_by_path
        .get(mdx.as_str())
        .or_else(|| page_by_path.get(md.as_str()))
        .copied()
}

fn build_folder_node(
    slug: &str,
    sub_dir: &str,
    meta: Option<&MetaEntry>,
    source: &SiteSource,
    page_by_path: &HashMap<&str, &CompiledPage>,
) -> NavNode {
    let index_page = find_dir_index(sub_dir, page_by_path);
    let title = meta
        .and_then(|m| m.title.clone())
        .or_else(|| index_page.map(|p| p.title.clone()))
        .unwrap_or_else(|| titleize_slug(slug));
    let children = build_dir_nodes(sub_dir, source, page_by_path);
    NavNode::Folder {
        title,
        slug: slug.to_string(),
        index_route: index_page.map(|p| p.route.clone()),
        collapsed: meta.is_some_and(|m| m.collapsed),
        badge: meta
            .and_then(|m| m.badge.clone())
            .or_else(|| index_page.and_then(|p| p.frontmatter.badge.clone())),
        icon: meta
            .and_then(|m| m.icon.clone())
            .or_else(|| index_page.and_then(|p| p.frontmatter.icon.clone())),
        children,
    }
}

fn flatten_nav_links(nodes: &[NavNode], out: &mut Vec<NavLink>) {
    for node in nodes {
        match node {
            NavNode::Page { title, route, .. } => {
                if !out.iter().any(|l| l.route == *route) {
                    out.push(NavLink {
                        title: title.clone(),
                        route: route.clone(),
                    });
                }
            }
            NavNode::Folder {
                title,
                index_route,
                children,
                ..
            } => {
                if let Some(route) = index_route
                    && !out.iter().any(|l| l.route == *route)
                {
                    out.push(NavLink {
                        title: title.clone(),
                        route: route.clone(),
                    });
                }
                flatten_nav_links(children, out);
            }
            NavNode::Separator { .. } | NavNode::ExternalLink { .. } => {}
        }
    }
}

fn titleize_slug(slug: &str) -> String {
    slug.split(['-', '_'])
        .filter(|s| !s.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
