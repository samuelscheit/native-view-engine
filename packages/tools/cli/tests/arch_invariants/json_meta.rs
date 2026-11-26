//! Std-only recursive-descent JSON parser and typed `WorkspaceGraph` built from
//! `cargo metadata --format-version 1 --offline --locked`.
//!
//! Replaces hand-written `Cargo.toml` line parsing so workspace-inherited deps
//! (`foo.workspace = true`), table deps (`[dependencies.foo]`), renamed packages
//! (`alias = { package = "real" }`), `[build-dependencies]`, optional deps, and transitive
//! dependency closures are resolved authoritatively by Cargo itself.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, PartialEq)]
pub enum JsonVal {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<JsonVal>),
    Obj(BTreeMap<String, JsonVal>),
}

impl JsonVal {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_arr(&self) -> Option<&[JsonVal]> {
        match self {
            Self::Arr(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_obj(&self) -> Option<&BTreeMap<String, JsonVal>> {
        match self {
            Self::Obj(m) => Some(m),
            _ => None,
        }
    }

    pub fn get(&self, key: &str) -> Option<&JsonVal> {
        self.as_obj()?.get(key)
    }
}

pub fn parse_json(input: &str) -> Result<JsonVal, String> {
    let mut p = JsonParser {
        bytes: input.as_bytes(),
        pos: 0,
    };
    let val = p.parse_value()?;
    p.skip_ws();
    if p.pos != p.bytes.len() {
        return Err(format!("trailing bytes at offset {}", p.pos));
    }
    Ok(val)
}

struct JsonParser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> JsonParser<'a> {
    fn skip_ws(&mut self) {
        while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn parse_value(&mut self) -> Result<JsonVal, String> {
        self.skip_ws();
        match self.peek() {
            Some(b'n') => self.expect_lit(b"null", JsonVal::Null),
            Some(b't') => self.expect_lit(b"true", JsonVal::Bool(true)),
            Some(b'f') => self.expect_lit(b"false", JsonVal::Bool(false)),
            Some(b'"') => self.parse_string().map(JsonVal::Str),
            Some(b'[') => self.parse_array(),
            Some(b'{') => self.parse_object(),
            Some(b'-' | b'0'..=b'9') => self.parse_number(),
            other => Err(format!("unexpected byte {:?} at offset {}", other, self.pos)),
        }
    }

    fn expect_lit(&mut self, lit: &[u8], val: JsonVal) -> Result<JsonVal, String> {
        if self.bytes[self.pos..].starts_with(lit) {
            self.pos += lit.len();
            Ok(val)
        } else {
            Err(format!("expected {:?} at offset {}", lit, self.pos))
        }
    }

    fn parse_string(&mut self) -> Result<String, String> {
        if self.peek() != Some(b'"') {
            return Err(format!("expected '\"' at offset {}", self.pos));
        }
        self.pos += 1;
        let mut out = String::new();
        while self.pos < self.bytes.len() {
            let b = self.bytes[self.pos];
            if b == b'"' {
                self.pos += 1;
                return Ok(out);
            }
            if b == b'\\' {
                self.pos += 1;
                let esc = *self
                    .bytes
                    .get(self.pos)
                    .ok_or_else(|| "unterminated escape".to_string())?;
                self.pos += 1;
                match esc {
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    b'/' => out.push('/'),
                    b'b' => out.push('\u{0008}'),
                    b'f' => out.push('\u{000C}'),
                    b'n' => out.push('\n'),
                    b'r' => out.push('\r'),
                    b't' => out.push('\t'),
                    b'u' => {
                        let cp = self.parse_hex4()?;
                        if (0xD800..=0xDBFF).contains(&cp) {
                            if !self.bytes[self.pos..].starts_with(b"\\u") {
                                return Err("unpaired UTF-16 high surrogate".to_string());
                            }
                            self.pos += 2;
                            let low = self.parse_hex4()?;
                            let combined =
                                0x10000 + (((cp as u32 - 0xD800) << 10) | (low as u32 - 0xDC00));
                            let ch = char::from_u32(combined)
                                .ok_or_else(|| "invalid surrogate pair".to_string())?;
                            out.push(ch);
                        } else {
                            let ch = char::from_u32(cp as u32)
                                .ok_or_else(|| "invalid unicode escape".to_string())?;
                            out.push(ch);
                        }
                    }
                    other => return Err(format!("invalid escape '\\{}'", other as char)),
                }
            } else if b < 0x80 {
                out.push(b as char);
                self.pos += 1;
            } else {
                let s = std::str::from_utf8(&self.bytes[self.pos..])
                    .map_err(|e| format!("invalid utf-8: {e}"))?;
                let ch = s.chars().next().ok_or_else(|| "eof in utf-8".to_string())?;
                out.push(ch);
                self.pos += ch.len_utf8();
            }
        }
        Err("unterminated JSON string".to_string())
    }

    fn parse_hex4(&mut self) -> Result<u16, String> {
        if self.pos + 4 > self.bytes.len() {
            return Err("truncated \\u escape".to_string());
        }
        let hex = std::str::from_utf8(&self.bytes[self.pos..self.pos + 4])
            .map_err(|e| e.to_string())?;
        self.pos += 4;
        u16::from_str_radix(hex, 16).map_err(|e| format!("bad \\u{hex}: {e}"))
    }

    fn parse_number(&mut self) -> Result<JsonVal, String> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        while matches!(
            self.peek(),
            Some(b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-')
        ) {
            self.pos += 1;
        }
        let slice = std::str::from_utf8(&self.bytes[start..self.pos]).map_err(|e| e.to_string())?;
        let n: f64 = slice
            .parse()
            .map_err(|e| format!("bad number `{slice}`: {e}"))?;
        Ok(JsonVal::Num(n))
    }

    fn parse_array(&mut self) -> Result<JsonVal, String> {
        self.pos += 1; // '['
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(JsonVal::Arr(items));
        }
        loop {
            items.push(self.parse_value()?);
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(JsonVal::Arr(items));
                }
                other => return Err(format!("expected ',' or ']' got {:?}", other)),
            }
        }
    }

    fn parse_object(&mut self) -> Result<JsonVal, String> {
        self.pos += 1; // '{'
        let mut map = BTreeMap::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(JsonVal::Obj(map));
        }
        loop {
            self.skip_ws();
            let key = self.parse_string()?;
            self.skip_ws();
            if self.peek() != Some(b':') {
                return Err(format!("expected ':' at offset {}", self.pos));
            }
            self.pos += 1;
            let val = self.parse_value()?;
            map.insert(key, val);
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(JsonVal::Obj(map));
                }
                other => return Err(format!("expected ',' or '}}' got {:?}", other)),
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepKind {
    Normal,
    Build,
    Dev,
}

#[derive(Debug, Clone)]
pub struct DeclaredDep {
    /// Canonical crate name (`package` field if renamed in Cargo.toml).
    pub name: String,
    pub kind: DepKind,
    pub target: Option<String>,
    /// Workspace-relative manifest path if this dep points to a path inside the workspace.
    pub target_rel_dir: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ResolvedEdge {
    pub pkg_id: String,
    pub kind: DepKind,
    pub target: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PackageMeta {
    pub id: String,
    pub name: String,
    pub rel_manifest: String,
    pub rel_dir: String,
    pub is_workspace: bool,
    pub is_proc_macro: bool,
    pub has_bench_target: bool,
    pub features: BTreeSet<String>,
    pub declared_deps: Vec<DeclaredDep>,
}

#[derive(Debug)]
pub struct WorkspaceGraph {
    pub packages: BTreeMap<String, PackageMeta>,
    pub workspace_ids: Vec<String>,
    pub resolve_edges: BTreeMap<String, Vec<ResolvedEdge>>,
    ws_id_by_name: BTreeMap<String, String>,
}

impl WorkspaceGraph {
    pub fn load_from_cargo(root: &Path) -> Self {
        let output = Command::new("cargo")
            .args(["metadata", "--format-version", "1", "--offline", "--locked"])
            .current_dir(root)
            .output()
            .expect("failed to run `cargo metadata --format-version 1 --offline --locked`");
        assert!(
            output.status.success(),
            "`cargo metadata` failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let raw = String::from_utf8(output.stdout).expect("cargo metadata stdout must be UTF-8");
        Self::parse_metadata_json(&raw).expect("failed to parse cargo metadata JSON")
    }

    pub fn parse_metadata_json(raw: &str) -> Result<Self, String> {
        let doc = parse_json(raw)?;
        let ws_root = doc
            .get("workspace_root")
            .and_then(JsonVal::as_str)
            .ok_or("missing workspace_root")?
            .replace('\\', "/");
        let ws_root_prefix = format!("{}/", ws_root.trim_end_matches('/'));

        let ws_member_set: BTreeSet<String> = doc
            .get("workspace_members")
            .and_then(JsonVal::as_arr)
            .ok_or("missing workspace_members")?
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect();

        let mut packages = BTreeMap::new();
        let mut ws_id_by_name = BTreeMap::new();

        for pkg in doc
            .get("packages")
            .and_then(JsonVal::as_arr)
            .ok_or("missing packages")?
        {
            let id = pkg
                .get("id")
                .and_then(JsonVal::as_str)
                .ok_or("pkg missing id")?
                .to_string();
            let name = pkg
                .get("name")
                .and_then(JsonVal::as_str)
                .ok_or("pkg missing name")?
                .to_string();
            let manifest_raw = pkg
                .get("manifest_path")
                .and_then(JsonVal::as_str)
                .unwrap_or("")
                .replace('\\', "/");
            let rel_manifest = manifest_raw
                .strip_prefix(&ws_root_prefix)
                .unwrap_or(&manifest_raw)
                .to_string();
            let rel_dir = rel_manifest
                .strip_suffix("/Cargo.toml")
                .unwrap_or(&rel_manifest)
                .to_string();
            let is_workspace = ws_member_set.contains(&id);

            let has_bench_target = pkg
                .get("targets")
                .and_then(JsonVal::as_arr)
                .is_some_and(|targets| {
                    targets.iter().any(|t| {
                        t.get("kind")
                            .and_then(JsonVal::as_arr)
                            .is_some_and(|kinds| kinds.iter().any(|k| k.as_str() == Some("bench")))
                    })
                });
            let is_proc_macro = pkg
                .get("targets")
                .and_then(JsonVal::as_arr)
                .is_some_and(|targets| {
                    targets.iter().any(|t| {
                        t.get("kind")
                            .and_then(JsonVal::as_arr)
                            .is_some_and(|kinds| {
                                kinds.iter().any(|k| k.as_str() == Some("proc-macro"))
                            })
                    })
                });

            let features: BTreeSet<String> = pkg
                .get("features")
                .and_then(JsonVal::as_obj)
                .map(|m| m.keys().cloned().collect())
                .unwrap_or_default();

            let mut declared_deps = Vec::new();
            if let Some(deps) = pkg.get("dependencies").and_then(JsonVal::as_arr) {
                for d in deps {
                    let Some(dep_name) = d.get("name").and_then(JsonVal::as_str) else {
                        continue;
                    };
                    let kind = match d.get("kind").and_then(JsonVal::as_str) {
                        None => DepKind::Normal,
                        Some("build") => DepKind::Build,
                        Some("dev") => DepKind::Dev,
                        Some(_) => DepKind::Normal,
                    };
                    let target = d.get("target").and_then(JsonVal::as_str).map(str::to_string);
                    let target_rel_dir = d
                        .get("path")
                        .and_then(JsonVal::as_str)
                        .map(|p| p.replace('\\', "/"))
                        .map(|p| {
                            p.strip_prefix(&ws_root_prefix)
                                .unwrap_or(&p)
                                .trim_end_matches('/')
                                .to_string()
                        });
                    declared_deps.push(DeclaredDep {
                        name: dep_name.to_string(),
                        kind,
                        target,
                        target_rel_dir,
                    });
                }
            }

            if is_workspace {
                ws_id_by_name.insert(name.clone(), id.clone());
            }
            packages.insert(
                id.clone(),
                PackageMeta {
                    id,
                    name,
                    rel_manifest,
                    rel_dir,
                    is_workspace,
                    is_proc_macro,
                    has_bench_target,
                    features,
                    declared_deps,
                },
            );
        }

        let mut workspace_ids: Vec<String> = ws_member_set.into_iter().collect();
        workspace_ids.sort_by(|a, b| {
            let ma = packages.get(a).map(|p| p.rel_manifest.as_str()).unwrap_or("");
            let mb = packages.get(b).map(|p| p.rel_manifest.as_str()).unwrap_or("");
            ma.cmp(mb)
        });

        let mut resolve_edges: BTreeMap<String, Vec<ResolvedEdge>> = BTreeMap::new();
        if let Some(nodes) = doc
            .get("resolve")
            .and_then(|r| r.get("nodes"))
            .and_then(JsonVal::as_arr)
        {
            for node in nodes {
                let Some(nid) = node.get("id").and_then(JsonVal::as_str) else {
                    continue;
                };
                let mut edges = Vec::new();
                if let Some(deps) = node.get("deps").and_then(JsonVal::as_arr) {
                    for d in deps {
                        let Some(target_pkg) = d.get("pkg").and_then(JsonVal::as_str) else {
                            continue;
                        };
                        if let Some(dks) = d.get("dep_kinds").and_then(JsonVal::as_arr) {
                            for dk in dks {
                                let kind = match dk.get("kind").and_then(JsonVal::as_str) {
                                    None => DepKind::Normal,
                                    Some("build") => DepKind::Build,
                                    Some("dev") => DepKind::Dev,
                                    Some(_) => DepKind::Normal,
                                };
                                let target = dk
                                    .get("target")
                                    .and_then(JsonVal::as_str)
                                    .map(str::to_string);
                                edges.push(ResolvedEdge {
                                    pkg_id: target_pkg.to_string(),
                                    kind,
                                    target,
                                });
                            }
                        }
                    }
                }
                resolve_edges.insert(nid.to_string(), edges);
            }
        }

        Ok(Self {
            packages,
            workspace_ids,
            resolve_edges,
            ws_id_by_name,
        })
    }

    pub fn workspace_packages(&self) -> impl Iterator<Item = &PackageMeta> {
        self.workspace_ids
            .iter()
            .filter_map(|id| self.packages.get(id))
    }

    pub fn workspace_pkg_by_name(&self, name: &str) -> Option<&PackageMeta> {
        let id = self.ws_id_by_name.get(name)?;
        self.packages.get(id)
    }

    /// Returns all transitive workspace packages reachable from `root_id` via declared
    /// normal+build dependencies (including optional deps), paired with the hop chain.
    pub fn transitive_workspace_deps(
        &self,
        root_id: &str,
        include_dev_at_root: bool,
    ) -> Vec<(&PackageMeta, Vec<String>)> {
        let Some(root) = self.packages.get(root_id) else {
            return Vec::new();
        };
        let mut visited: BTreeSet<String> = BTreeSet::new();
        visited.insert(root_id.to_string());
        let mut queue: VecDeque<(&PackageMeta, Vec<String>, bool)> = VecDeque::new();
        queue.push_back((root, vec![root.name.clone()], true));
        let mut results = Vec::new();

        while let Some((cur, path, is_root)) = queue.pop_front() {
            for dep in &cur.declared_deps {
                let allowed_kind = dep.kind != DepKind::Dev || (is_root && include_dev_at_root);
                if !allowed_kind {
                    continue;
                }
                let target_ws = self.workspace_pkg_by_name(&dep.name).or_else(|| {
                    let rel = dep.target_rel_dir.as_deref()?;
                    self.workspace_packages().find(|p| p.rel_dir == rel)
                });
                let Some(next) = target_ws else {
                    continue;
                };
                if visited.insert(next.id.clone()) {
                    let mut next_path = path.clone();
                    next_path.push(next.name.clone());
                    results.push((next, next_path.clone()));
                    queue.push_back((next, next_path, false));
                }
            }
        }
        results
    }

    /// Returns all direct AND transitive non-dev crate names reachable from `root_id` via
    /// normal + build dependencies, mapped to the shortest dependency chain.
    ///
    /// Includes ALL direct declared normal+build deps (including optional/renamed/build) and
    /// walks `resolve.nodes` skipping non-compiled formal-verification edges (`cfg(hax)`) and
    /// cross-target wasm edges inside third-party native-only crates (`getrandom`, `cc`).
    pub fn transitive_non_dev_crates(&self, root_id: &str) -> BTreeMap<String, Vec<String>> {
        let Some(root) = self.packages.get(root_id) else {
            return BTreeMap::new();
        };
        let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();

        // 1. Always include every declared normal/build dep of `root` AND every transitive
        //    workspace package's declared normal/build deps (catches optional + renamed deps!).
        let mut ws_frontier: Vec<(&PackageMeta, Vec<String>)> =
            vec![(root, vec![root.name.clone()])];
        ws_frontier.extend(self.transitive_workspace_deps(root_id, false));
        for (ws_pkg, ws_path) in &ws_frontier {
            for dep in &ws_pkg.declared_deps {
                if dep.kind == DepKind::Dev {
                    continue;
                }
                out.entry(dep.name.clone()).or_insert_with(|| {
                    let mut p = ws_path.clone();
                    p.push(dep.name.clone());
                    p
                });
            }
        }

        // 2. Walk Cargo's resolved graph for transitive third-party normal+build deps.
        let mut visited: BTreeSet<String> = BTreeSet::new();
        let mut queue: VecDeque<(String, Vec<String>, bool)> = VecDeque::new();
        visited.insert(root_id.to_string());
        queue.push_back((root_id.to_string(), vec![root.name.clone()], false));

        while let Some((cur_id, path, via_build)) = queue.pop_front() {
            let Some(edges) = self.resolve_edges.get(&cur_id) else {
                continue;
            };
            let cur_is_ws = self
                .packages
                .get(&cur_id)
                .is_some_and(|p| p.is_workspace);
            for edge in edges {
                if edge.kind == DepKind::Dev {
                    continue;
                }
                if is_unreachable_third_party_cfg(cur_is_ws, via_build, edge.target.as_deref()) {
                    continue;
                }
                let Some(next_pkg) = self.packages.get(&edge.pkg_id) else {
                    continue;
                };
                let mut next_path = path.clone();
                next_path.push(next_pkg.name.clone());
                out.entry(next_pkg.name.clone())
                    .or_insert_with(|| next_path.clone());
                if visited.insert(edge.pkg_id.clone()) {
                    queue.push_back((
                        edge.pkg_id.clone(),
                        next_path,
                        via_build || edge.kind == DepKind::Build,
                    ));
                }
            }
        }
        out
    }
}

/// Filters out Cargo `resolve.nodes` edges that are never compiled in actual host/WASM builds
/// (specifically third-party `cfg(hax)` formal verification hooks and `cfg(target_family = "wasm")`
/// edges inside third-party crates like `getrandom` / build-script transitive graphs).
/// Workspace-authored edges (`cur_is_ws == true`) are NEVER filtered.
fn is_unreachable_third_party_cfg(
    cur_is_ws: bool,
    via_build: bool,
    target: Option<&str>,
) -> bool {
    if cur_is_ws {
        return false;
    }
    let Some(cfg) = target else {
        return false;
    };
    if cfg.contains("hax") {
        return true;
    }
    if cfg.contains("target_family = \"wasm\"") || cfg.contains("target_arch = \"wasm32\"") {
        return via_build || cfg.contains("target_os = \"unknown\"");
    }
    false
}
