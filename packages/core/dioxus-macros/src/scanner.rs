//! Source-file scanner assigning file-order template ordinals matching `nve-cli`'s `rsx_diff`
//! parser 1-to-1 across every `rsx!`, `if`, `else if`, `else`, and `for` body.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

pub(crate) use crate::lexer::LineCol;
use crate::lexer::{Lex, Tok};

/// Cached template anchor lookup tables for one `.rs` source file.
#[derive(Debug)]
pub(crate) struct FileTemplates {
    pub(crate) rel_file: String,
    /// `rsx! { ... }` closing-delimiter end `(line, col)` -> template ordinal.
    pub(crate) rsx_by_end: HashMap<LineCol, u32>,
    /// `rsx` identifier start `(line, col)` -> template ordinal.
    pub(crate) rsx_by_start: HashMap<LineCol, u32>,
    /// `if` / `else if` / `else` / `for` body `{` start `(line, col)` -> template ordinal.
    pub(crate) body_by_open: HashMap<LineCol, u32>,
    /// `if` / `else if` / `else` / `for` body `}` end `(line, col)` -> template ordinal.
    pub(crate) body_by_end: HashMap<LineCol, u32>,
}

impl FileTemplates {
    pub(crate) fn rsx_ordinal(&self, start: LineCol, end: LineCol) -> Option<u32> {
        self.rsx_by_end
            .get(&end)
            .or_else(|| self.rsx_by_start.get(&start))
            .copied()
    }

    pub(crate) fn body_ordinal(&self, open: LineCol, end: LineCol) -> Option<u32> {
        self.body_by_open
            .get(&open)
            .or_else(|| self.body_by_end.get(&end))
            .copied()
    }
}

/// Returns the cached [`FileTemplates`] for `local_path` (normalized relative label from `raw_file`).
pub(crate) fn templates_for_file(
    local_path: &Path,
    raw_file: &str,
) -> Option<&'static FileTemplates> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, &'static FileTemplates>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = cache.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(&cached) = guard.get(local_path) {
        return Some(cached);
    }
    let src = std::fs::read_to_string(local_path).ok()?;
    let rel_file = normalize_rel_file(local_path, raw_file);
    let parsed = Box::leak(Box::new(scan_file(&src, rel_file)));
    guard.insert(local_path.to_path_buf(), parsed);
    Some(parsed)
}

fn normalize_rel_file(local_path: &Path, raw_file: &str) -> String {
    let raw_path = Path::new(raw_file);
    let candidate = if !raw_file.is_empty() && !raw_path.is_absolute() {
        raw_file.to_owned()
    } else if let Some(root) = find_workspace_root(local_path)
        && let Ok(stripped) = local_path.strip_prefix(&root)
    {
        stripped.to_string_lossy().into_owned()
    } else if !raw_file.is_empty() {
        raw_file.to_owned()
    } else {
        local_path.to_string_lossy().into_owned()
    };
    let mut norm = candidate.replace('\\', "/");
    while let Some(rest) = norm.strip_prefix("./") {
        norm = rest.to_owned();
    }
    norm
}

fn find_workspace_root(path: &Path) -> Option<PathBuf> {
    let mut cur = path.parent();
    while let Some(dir) = cur {
        let manifest = dir.join("Cargo.toml");
        if manifest.is_file()
            && let Ok(text) = std::fs::read_to_string(&manifest)
            && text.contains("[workspace]")
        {
            return Some(dir.to_path_buf());
        }
        cur = dir.parent();
    }
    None
}

// ============================================================================
// Template ordinal scanner (mirrors `rsx_diff/parser.rs` 1-to-1)
// ============================================================================

pub(crate) fn is_rust_keyword(ident: &str) -> bool {
    matches!(
        ident,
        "if" | "else"
            | "for"
            | "match"
            | "while"
            | "loop"
            | "let"
            | "move"
            | "async"
            | "unsafe"
            | "return"
            | "fn"
            | "struct"
            | "enum"
            | "impl"
            | "mod"
            | "use"
            | "in"
            | "const"
            | "break"
    )
}

pub(crate) fn is_native_tag(tag: &str) -> bool {
    matches!(
        tag,
        "view"
            | "div"
            | "text"
            | "span"
            | "p"
            | "a"
            | "pressable"
            | "button"
            | "scrollview"
            | "image"
            | "textinput"
            | "input"
    )
}

fn scan_file(src: &str, rel_file: String) -> FileTemplates {
    let mut out = FileTemplates {
        rel_file,
        rsx_by_end: HashMap::new(),
        rsx_by_start: HashMap::new(),
        body_by_open: HashMap::new(),
        body_by_end: HashMap::new(),
    };
    let Some(lex) = Lex::tokenize(src) else {
        return out;
    };
    let mut scanner = FileScanner {
        lex: &lex,
        out: &mut out,
        next_ordinal: 0,
    };
    scanner.scan_rust_range(0, lex.len());
    out
}

struct FileScanner<'a> {
    lex: &'a Lex<'a>,
    out: &'a mut FileTemplates,
    next_ordinal: u32,
}

impl FileScanner<'_> {
    fn is_rsx_macro(&self, pos: usize, end: usize) -> bool {
        pos + 2 < end
            && self.lex.is_ident(pos, "rsx")
            && self.lex.is_punct(pos + 1, b'!')
            && self.lex.is_punct(pos + 2, b'{')
    }

    fn scan_rsx_template(&mut self, rsx_tok: usize, open: usize) -> usize {
        let close = self.lex.close(open);
        let ord = self.next_ordinal;
        self.next_ordinal += 1;
        self.out.rsx_by_start.insert(self.lex.tok_start_lc(rsx_tok), ord);
        self.out.rsx_by_end.insert(self.lex.tok_end_lc(close), ord);
        self.scan_rsx_children(open + 1, close);
        close + 1
    }

    fn scan_body_template(&mut self, open: usize) -> usize {
        let close = self.lex.close(open);
        let ord = self.next_ordinal;
        self.next_ordinal += 1;
        self.out.body_by_open.insert(self.lex.tok_start_lc(open), ord);
        self.out.body_by_end.insert(self.lex.tok_end_lc(close), ord);
        self.scan_rsx_children(open + 1, close);
        close + 1
    }

    fn scan_rust_range(&mut self, start: usize, end: usize) {
        let mut pos = start;
        while pos + 2 < end {
            if self.is_rsx_macro(pos, end) && self.lex.close(pos + 2) < end {
                pos = self.scan_rsx_template(pos, pos + 2);
                continue;
            }
            pos += 1;
        }
    }

    fn scan_rsx_children(&mut self, start: usize, end: usize) {
        let mut pos = start;
        while pos < end {
            pos = self.scan_one_rsx_child(pos, end);
        }
    }

    fn scan_if_chain(&mut self, mut pos: usize, end: usize) -> usize {
        loop {
            let Some(body) = find_body_brace(self.lex, pos, end, false) else {
                self.scan_rust_range(pos, end);
                return end;
            };
            self.scan_rust_range(pos, body);
            pos = self.scan_body_template(body);
            if !self.lex.is_ident(pos, "else") || pos >= end {
                return pos;
            }
            pos += 1;
            if self.lex.is_ident(pos, "if") {
                pos += 1;
            } else if self.lex.is_punct(pos, b'{') {
                return self.scan_body_template(pos);
            } else {
                return pos;
            }
        }
    }

    fn scan_one_rsx_child(&mut self, pos: usize, end: usize) -> usize {
        let lex = self.lex;
        if lex.is_punct(pos, b',') || lex.is_punct(pos, b';') {
            return pos + 1;
        }
        if self.is_rsx_macro(pos, end) {
            return self.scan_rsx_template(pos, pos + 2);
        }
        if lex.is_ident(pos, "if") {
            return self.scan_if_chain(pos + 1, end);
        }
        if lex.is_ident(pos, "for") {
            let Some(body) = find_body_brace(lex, pos + 1, end, true) else {
                self.scan_rust_range(pos + 1, end);
                return end;
            };
            self.scan_rust_range(pos + 1, body);
            return self.scan_body_template(body);
        }
        if lex.is_ident(pos, "match") {
            let Some(arms) = find_body_brace(lex, pos + 1, end, false) else {
                self.scan_rust_range(pos + 1, end);
                return end;
            };
            let close = lex.close(arms);
            self.scan_rust_range(pos + 1, close);
            return close + 1;
        }
        if lex.is_punct(pos, b'{') {
            let close = lex.close(pos);
            self.scan_rust_range(pos + 1, close);
            return close + 1;
        }
        if let Some(tag) = lex.ident(pos)
            && !is_rust_keyword(tag)
            && lex.is_punct(pos + 1, b'{')
            && pos + 1 < end
        {
            let close = self.scan_rsx_element(pos + 1);
            return close + 1;
        }
        if matches!(lex.kind(pos), Tok::Str | Tok::RawStr) && !lex.is_punct(pos + 1, b':') {
            return pos + 1;
        }
        if lex.is_punct(pos, b'(') || lex.is_punct(pos, b'[') {
            let close = lex.close(pos);
            self.scan_rust_range(pos + 1, close);
            return close + 1;
        }
        pos + 1
    }

    fn scan_rsx_element(&mut self, open: usize) -> usize {
        let lex = self.lex;
        let close = lex.close(open);
        let mut pos = open + 1;
        while pos < close {
            if let Some((val_start, val_end, next)) = parse_rsx_attr(lex, pos, close) {
                self.scan_rust_range(val_start, val_end);
                pos = next;
                continue;
            }
            if lex.ident(pos).is_some_and(|id| !is_rust_keyword(id)) && lex.is_punct(pos + 1, b',')
            {
                pos += 2;
                continue;
            }
            pos = self.scan_one_rsx_child(pos, close);
        }
        close
    }
}

fn parse_rsx_attr(lex: &Lex<'_>, pos: usize, end: usize) -> Option<(usize, usize, usize)> {
    if pos + 2 >= end || !lex.is_punct(pos + 1, b':') || lex.is_punct(pos + 2, b':') {
        return None;
    }
    match lex.kind(pos) {
        Tok::Ident if !is_rust_keyword(lex.text(pos)) => {}
        Tok::Str | Tok::RawStr => {}
        _ => return None,
    }
    let val_end = value_span_end(lex, pos + 2, end);
    let next = val_end + usize::from(val_end < end && lex.is_punct(val_end, b','));
    Some((pos + 2, val_end, next))
}

fn value_span_end(lex: &Lex<'_>, start: usize, limit: usize) -> usize {
    let mut cur = start;
    while cur < limit {
        if lex.kind(cur) == Tok::Punct {
            match lex.src.as_bytes()[lex.tokens[cur].start] {
                b',' | b'}' | b')' | b']' => return cur,
                b'{' | b'(' | b'[' => {
                    cur = lex.close(cur) + 1;
                    continue;
                }
                _ => {}
            }
        }
        cur += 1;
    }
    limit
}

fn skip_tok_or_group(lex: &Lex<'_>, pos: usize) -> usize {
    if lex.is_punct(pos, b'{') || lex.is_punct(pos, b'(') || lex.is_punct(pos, b'[') {
        lex.close(pos) + 1
    } else {
        pos + 1
    }
}

fn expects_operand(lex: &Lex<'_>, i: usize) -> bool {
    match lex.kind(i) {
        Tok::Punct => !matches!(lex.src.as_bytes()[lex.tokens[i].start], b')' | b']' | b'}' | b'?'),
        Tok::Ident => matches!(
            lex.text(i),
            "if" | "in"
                | "return"
                | "move"
                | "unsafe"
                | "async"
                | "loop"
                | "const"
                | "break"
                | "match"
        ),
        _ => false,
    }
}

fn continues_expr(lex: &Lex<'_>, k: usize, end: usize) -> bool {
    k < end
        && (lex.is_ident(k, "as")
            || (lex.kind(k) == Tok::Punct
                && matches!(
                    lex.src.as_bytes()[lex.tokens[k].start],
                    b'.' | b'?'
                        | b'='
                        | b'!'
                        | b'<'
                        | b'>'
                        | b'+'
                        | b'-'
                        | b'*'
                        | b'/'
                        | b'%'
                        | b'^'
                        | b'&'
                        | b'|'
                )))
}

fn find_let_eq(lex: &Lex<'_>, mut cur: usize, end: usize) -> Option<usize> {
    while cur < end {
        if lex.is_punct(cur, b'=')
            && !lex.is_punct(cur + 1, b'=')
            && !lex.is_punct(cur + 1, b'>')
            && !(cur > 0
                && lex.kind(cur - 1) == Tok::Punct
                && b"=!<>+-*/%^&|.".contains(&lex.src.as_bytes()[lex.tokens[cur - 1].start]))
        {
            return Some(cur);
        }
        cur = skip_tok_or_group(lex, cur);
    }
    None
}

fn find_body_brace(lex: &Lex<'_>, start: usize, end: usize, is_for: bool) -> Option<usize> {
    let mut cur = start;
    if is_for {
        while !lex.is_ident(cur, "in") {
            if cur >= end {
                return None;
            }
            cur = skip_tok_or_group(lex, cur);
        }
        cur += 1;
    }
    let mut operand_expected = true;
    let mut pending_match_arms = 0u32;
    while cur < end {
        if lex.is_ident(cur, "let") {
            cur = find_let_eq(lex, cur + 1, end)? + 1;
            operand_expected = true;
            continue;
        }
        if lex.is_punct(cur, b'{') {
            let close = lex.close(cur);
            let is_head = if operand_expected {
                true
            } else if pending_match_arms > 0 {
                pending_match_arms -= 1;
                true
            } else {
                continues_expr(lex, close + 1, end)
            };
            if !is_head {
                return Some(cur);
            }
            cur = close + 1;
            operand_expected = false;
            continue;
        }
        if lex.is_ident(cur, "match") {
            pending_match_arms += 1;
        }
        let is_group = lex.is_punct(cur, b'(') || lex.is_punct(cur, b'[');
        operand_expected = !is_group && expects_operand(lex, cur);
        cur = skip_tok_or_group(lex, cur);
    }
    None
}
