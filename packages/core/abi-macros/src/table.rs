//! `#[derive(NveTable)]`: word-table schema → zero-cost typed rows for `nve-store`.
//!
//! ```ignore
//! #[derive(NveTable)]
//! #[nve_table(name = "nve.visual", storage = "dense")]
//! pub struct Visual {
//!     #[effect(paint)] pub background: Rgba,
//!     #[effect(composite)] #[default(1.0)] pub opacity: f32,
//!     #[effect(paint)] #[bits(2)] pub border_style: BorderStyle,
//!     #[effect(semantics)] pub label: Str,
//!     #[effect(paint)] pub shadows: Res<BoxShadows>,
//! }
//! ```
//! Fields without `#[bits]` take whole words; `#[bits(n)]` fields pack consecutively into shared
//! words of one effect class; `Str` takes two words. Emits `const DESC`, `Prop` constants
//! (`Visual::OPACITY`), `VisualRef<'a>` getters and fluent `VisualMut<'a>` setters.

use proc_macro::{Delimiter, TokenStream, TokenTree};

use crate::emit::quote_str;
use crate::parse::{
    expect_ident, is_delim, is_punct, parse_string_literal, parse_usize_literal, skip_vis,
};

const S: &str = "::nve_store";

enum Class {
    Scalar,
    Str,
    Res(String),
}

struct Field {
    name: String,
    ty: String,
    short_ty: String,
    class: Class,
    effects: u8,
    default: Option<String>,
    bits: Option<u32>,
    doc: Vec<String>,
    word: u32,
    shift: u32,
    width: u32,
}

struct Table {
    vis: String,
    ident: String,
    name: String,
    dense: bool,
    flags: u8,
    fields: Vec<Field>,
}

pub(crate) fn derive(input: TokenStream) -> Result<String, String> {
    let mut t = parse(input)?;
    layout(&mut t)?;
    Ok(emit(&t))
}

fn effect_bit(name: &str) -> Result<u8, String> {
    Ok(match name {
        "layout" => 1,
        "measure" => 2,
        "paint" => 4,
        "composite" => 8,
        "semantics" => 16,
        "realize" => 32,
        "events" => 64,
        other => return Err(format!("unknown effect `{other}` (layout|measure|paint|composite|semantics|realize|events)")),
    })
}

fn group_inner(rest: &[TokenTree]) -> Option<Vec<TokenTree>> {
    match rest.first() {
        Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Parenthesis => {
            Some(g.stream().into_iter().collect())
        }
        _ => None,
    }
}

fn parse(input: TokenStream) -> Result<Table, String> {
    let tokens: Vec<TokenTree> = input.into_iter().collect();
    let mut pos = 0;
    let (mut name, mut dense, mut flags) = (None, true, 0u8);
    while is_punct(&tokens, pos, '#') {
        if let Some(TokenTree::Group(g)) = tokens.get(pos + 1) {
            let inner: Vec<_> = g.stream().into_iter().collect();
            if matches!(inner.first(), Some(TokenTree::Ident(i)) if i.to_string() == "nve_table") {
                let kv = group_inner(&inner[1..]).ok_or("expected `#[nve_table(...)]`")?;
                let mut i = 0;
                while i < kv.len() {
                    let TokenTree::Ident(k) = &kv[i] else {
                        return Err(format!("expected key identifier in #[nve_table], found `{}`", kv[i]));
                    };
                    if !is_punct(&kv, i + 1, '=') {
                        return Err(format!("expected `=` after #[nve_table] key `{k}`"));
                    }
                    let TokenTree::Literal(v) = kv.get(i + 2).ok_or("expected value in #[nve_table]")? else {
                        return Err(format!("expected string literal for #[nve_table] key `{k}`"));
                    };
                    let v = parse_string_literal(&v.to_string())?;
                    match k.to_string().as_str() {
                        "name" => name = Some(v),
                        "storage" => {
                            dense = match v.as_str() {
                                "dense" => true,
                                "sparse" => false,
                                o => return Err(format!("storage must be dense|sparse, got `{o}`")),
                            }
                        }
                        "flags" => {
                            for f in v.split('|').map(str::trim).filter(|f| !f.is_empty()) {
                                flags |= match f {
                                    "host" => 1,
                                    "publish" => 2,
                                    "layout" => 4,
                                    o => return Err(format!("unknown table flag `{o}` (host|publish|layout)")),
                                };
                            }
                        }
                        o => return Err(format!("unknown #[nve_table] key `{o}`")),
                    }
                    i += 3;
                    if i < kv.len() {
                        if !is_punct(&kv, i, ',') {
                            return Err(format!("expected `,` in #[nve_table], found `{}`", kv[i]));
                        }
                        i += 1;
                    }
                }
            }
        }
        pos += 2;
    }
    let vis_start = pos;
    skip_vis(&tokens, &mut pos);
    let vis: String = tokens[vis_start..pos]
        .iter()
        .cloned()
        .collect::<TokenStream>()
        .to_string();
    let kw = expect_ident(&tokens, &mut pos, "struct")?;
    if kw != "struct" {
        return Err("#[derive(NveTable)] only supports structs".into());
    }
    let ident = expect_ident(&tokens, &mut pos, "struct name")?;
    let Some(TokenTree::Group(body)) = tokens.get(pos).filter(|t| is_delim(t, Delimiter::Brace)) else {
        return Err(format!("#[derive(NveTable)] requires named fields on `{ident}`"));
    };
    let name = name.ok_or_else(|| format!("`{ident}` needs #[nve_table(name = \"...\")]"))?;
    Ok(Table { vis, ident, name, dense, flags, fields: parse_fields(body.stream())? })
}

fn parse_fields(body: TokenStream) -> Result<Vec<Field>, String> {
    let tokens: Vec<TokenTree> = body.into_iter().collect();
    let (mut pos, mut out) = (0, Vec::new());
    while pos < tokens.len() {
        let (mut effects, mut default, mut bits, mut doc) = (0u8, None, None, Vec::new());
        while is_punct(&tokens, pos, '#') {
            let Some(TokenTree::Group(g)) = tokens.get(pos + 1) else { return Err("bad attribute".into()) };
            let inner: Vec<_> = g.stream().into_iter().collect();
            let attr = match inner.first() { Some(TokenTree::Ident(i)) => i.to_string(), _ => String::new() };
            match attr.as_str() {
                "effect" => {
                    for t in group_inner(&inner[1..]).ok_or("expected `#[effect(...)]`")? {
                        if let TokenTree::Ident(e) = t {
                            effects |= effect_bit(&e.to_string())?;
                        }
                    }
                }
                "default" => {
                    let g = group_inner(&inner[1..]).ok_or("expected `#[default(expr)]`")?;
                    default = Some(g.into_iter().collect::<TokenStream>().to_string());
                }
                "bits" => {
                    let g = group_inner(&inner[1..]).ok_or("expected `#[bits(n)]`")?;
                    let [TokenTree::Literal(l)] = g.as_slice() else { return Err("expected integer in #[bits]".into()) };
                    bits = Some(parse_usize_literal(&l.to_string())? as u32);
                }
                "doc" => if let Some(TokenTree::Literal(l)) = inner.get(2) {
                    doc.push(parse_string_literal(&l.to_string())?);
                },
                _ => {}
            }
            pos += 2;
        }
        if pos >= tokens.len() { break; }
        skip_vis(&tokens, &mut pos);
        let name = expect_ident(&tokens, &mut pos, "field name")?;
        if !is_punct(&tokens, pos, ':') { return Err(format!("expected `:` after `{name}`")); }
        pos += 1;
        let start = pos;
        let mut depth = 0i32;
        while pos < tokens.len() {
            match &tokens[pos] {
                TokenTree::Punct(p) if p.as_char() == '<' => depth += 1,
                TokenTree::Punct(p) if p.as_char() == '>' => depth -= 1,
                TokenTree::Punct(p) if p.as_char() == ',' && depth == 0 => break,
                _ => {}
            }
            pos += 1;
        }
        let ty_tokens: TokenStream = tokens[start..pos].iter().cloned().collect();
        let ty = ty_tokens.to_string();
        pos += 1;
        let compact: String = ty.chars().filter(|c| !c.is_whitespace()).collect();
        let outer_head = compact.split('<').next().unwrap_or(&compact);
        let outer_last = outer_head.rsplit("::").next().unwrap_or(outer_head).to_owned();
        let class = if outer_last == "Str" && !compact.contains('<') {
            Class::Str
        } else if outer_last == "Res"
            && let Some(lt) = compact.find('<')
            && compact.ends_with('>')
        {
            let inner = compact[lt + 1..compact.len() - 1].to_owned();
            if inner.is_empty() {
                return Err(format!("`{name}`: Res<T> requires a concrete resource type"));
            }
            Class::Res(inner)
        } else {
            Class::Scalar
        };
        if effects == 0 { return Err(format!("field `{name}` needs #[effect(...)]")); }
        if !matches!(class, Class::Scalar) && (default.is_some() || bits.is_some()) {
            return Err(format!("`{name}`: Str/Res fields take no #[default]/#[bits]"));
        }
        out.push(Field { name, ty, short_ty: outer_last, class, effects, default, bits, doc, word: 0, shift: 0, width: 32 });
    }
    Ok(out)
}

fn layout(t: &mut Table) -> Result<(), String> {
    let mut next = 0u32;
    // Open packed word: (word, used bits, effects).
    let mut open: Option<(u32, u32, u8)> = None;
    for f in &mut t.fields {
        match (&f.class, f.bits) {
            (Class::Scalar, Some(b)) => {
                if b == 0 || b > 32 { return Err(format!("`{}`: #[bits] must be 1..=32", f.name)); }
                match open {
                    Some((w, used, fx)) if used + b <= 32 && fx == f.effects => {
                        f.word = w; f.shift = used; f.width = b;
                        open = Some((w, used + b, fx));
                    }
                    _ => {
                        f.word = next; f.shift = 0; f.width = b;
                        open = Some((next, b, f.effects));
                        next += 1;
                    }
                }
            }
            (Class::Str, _) => { f.word = next; next += 2; }
            _ => { f.word = next; next += 1; }
        }
    }
    if next == 0 || next > 32 {
        return Err(format!("table `{}` has {next} words (must be 1..=32); split it", t.ident));
    }
    Ok(())
}

fn upper(s: &str) -> String {
    s.to_ascii_uppercase()
}

fn mask(f: &Field) -> String {
    if f.width >= 32 { "0xFFFF_FFFFu32".into() } else { format!("0x{:X}u32", ((1u64 << f.width) - 1) << f.shift) }
}

fn default_bits(f: &Field) -> Option<String> {
    let e = f.default.as_ref()?;
    Some(match f.short_ty.as_str() {
        "f32" => format!("f32::to_bits({e})"),
        "u32" | "u16" | "u8" | "i32" | "bool" => format!("(({e}) as u32)"),
        "i16" => format!("((({e}) as i16) as u16 as u32)"),
        "NodeId" => format!("(({e}).0)"),
        _ => format!("(({e}).to_word())"),
    })
}

fn docs(f: &Field) -> String {
    f.doc.iter().map(|d| format!("#[doc = {}]", quote_str(d))).collect()
}

fn emit(t: &Table) -> String {
    let id = &t.ident;
    let vis = &t.vis;
    let words = t.fields.iter().map(|f| f.word + if matches!(f.class, Class::Str) { 2 } else { 1 }).max().unwrap_or(0) as usize;
    let mut defaults = vec![String::from("0u32"); words];
    let mut fx = vec![0u8; words];
    let (mut str_words, mut res_words) = (0u32, 0u32);
    let (mut fields, mut props, mut getters, mut setters, mut regs, mut asserts) =
        (String::new(), String::new(), String::new(), String::new(), String::new(), String::new());
    for f in &t.fields {
        let (w, sh, m, ty, name) = (f.word, f.shift, mask(f), &f.ty, &f.name);
        let d = docs(f);
        fx[w as usize] |= f.effects;
        fields.push_str(&format!(
            "{S}::FieldDesc {{ name: {}, kind: <{ty} as {S}::TableField>::KIND, word: {w}, shift: {sh}, bits: {}, effects: {}, variants: <{ty} as {S}::TableField>::VARIANTS }},",
            quote_str(name), if matches!(f.class, Class::Str) { 64 } else { f.width }, f.effects));
        regs.push_str(&format!("<{ty} as {S}::TableField>::register();"));
        let cname = upper(name);
        match &f.class {
            Class::Scalar => {
                if let Some(b) = default_bits(f) {
                    defaults[w as usize].push_str(&format!(" | (({b} << {sh}) & {m})"));
                }
                if let Some(b) = f.bits {
                    asserts.push_str(&format!("const _: () = assert!((<{ty} as {S}::WordValue>::BITS as u32) <= {b}, \"#[bits] narrower than the field type\");"));
                }
                props.push_str(&format!("{d} pub const {cname}: {S}::Prop<{id}, {ty}> = unsafe {{ {S}::Prop::new({w}, {sh}, {}) }};", f.width));
                let read = if f.width >= 32 { format!("self.0.word({w})") } else { format!("(self.0.word({w}) & {m}) >> {sh}") };
                getters.push_str(&format!("{d} #[inline(always)] pub fn {name}(&self) -> {ty} {{ <{ty} as {S}::WordValue>::from_bits({read}) }}"));
                setters.push_str(&format!("{d} #[inline(always)] pub fn {name}(mut self, v: {ty}) -> Self {{ self.0.put({w}, {sh}, {m}, <{ty} as {S}::WordValue>::to_bits(v)); self }}"));
            }
            Class::Str => {
                str_words |= 1 << w;
                fx[w as usize + 1] |= f.effects;
                props.push_str(&format!("{d} pub const {cname}: {S}::StrProp<{id}> = unsafe {{ {S}::StrProp::new({w}) }};"));
                getters.push_str(&format!(
                    "{d} #[inline(always)] pub fn {name}(&self) -> &'a str {{ self.0.str_at({w}) }} \
                     #[doc = \"Raw `(off, len)` span into the store text slab.\"] #[inline(always)] pub fn {name}_span(&self) -> {S}::StrSpan {{ self.0.str_span_at({w}) }}"
                ));
                setters.push_str(&format!("{d} #[inline] pub fn {name}(mut self, v: &str) -> Self {{ self.0.put_str({w}, v); self }}"));
            }
            Class::Res(r) => {
                res_words |= 1 << w;
                props.push_str(&format!("{d} pub const {cname}: {S}::ResProp<{id}, {r}> = unsafe {{ {S}::ResProp::new({w}) }};"));
                getters.push_str(&format!("{d} #[inline(always)] pub fn {name}(&self) -> {S}::ResourceId {{ {S}::ResourceId(self.0.word({w})) }}"));
                setters.push_str(&format!(
                    "{d} #[inline] pub fn {name}(mut self, v: ::core::option::Option<{r}>) -> Self {{ self.0.put_resource({w}, v); self }} \
                     #[doc = \"Ephemeral (in-place, non-deduplicated) write; for per-frame blobs.\"] #[inline] pub fn {name}_ephemeral(mut self, v: {r}) -> Self {{ self.0.put_resource_ephemeral({w}, v); self }}"));
            }
        }
    }
    let mut flags = t.flags;
    if fx.iter().any(|&e| e & 3 != 0) { flags |= 4; }
    let storage = if t.dense { "Dense" } else { "Sparse" };
    let defaults = defaults.join(", ");
    let fx = fx.iter().map(|e| e.to_string()).collect::<Vec<_>>().join(", ");
    let name = quote_str(&t.name);
    format!(r#"
const _: () = {{
    const DEFAULTS: &[{S}::Word] = &[{defaults}];
    const WORD_EFFECTS: &[u8] = &[{fx}];
    const FIELDS: &[{S}::FieldDesc] = &[{fields}];
    unsafe impl {S}::NveTable for {id} {{
        const DESC: {S}::TableDesc = {S}::TableDesc {{
            key: {S}::__private::Key64::of({name}),
            name: {name},
            schema_hash: {S}::__private::compute_table_schema_hash(
                {name},
                {S}::Storage::{storage},
                {flags},
                DEFAULTS,
                WORD_EFFECTS,
                {str_words},
                {res_words},
                FIELDS,
            ),
            storage: {S}::Storage::{storage},
            flags: {flags},
            defaults: DEFAULTS,
            word_effects: WORD_EFFECTS,
            str_words: {str_words},
            resource_words: {res_words},
            fields: FIELDS,
        }};
        type Ref<'a> = {id}Ref<'a>;
        type Mut<'a> = {id}Mut<'a>;
        #[inline(always)]
        fn from_row_ref<'a>(row: {S}::RowRef<'a>) -> Self::Ref<'a> {{ {id}Ref(row) }}
        #[inline(always)]
        fn from_row_writer<'a>(writer: {S}::RowWriter<'a>) -> Self::Mut<'a> {{ {id}Mut(writer) }}
        #[inline(always)]
        fn slot() -> {S}::TableSlot {{
            static CELL: ::core::sync::atomic::AtomicU8 = ::core::sync::atomic::AtomicU8::new(u8::MAX);
            static DESC: {S}::TableDesc = <{id} as {S}::NveTable>::DESC;
            let s = CELL.load(::core::sync::atomic::Ordering::Acquire);
            if s != u8::MAX {{ {S}::TableSlot(s) }} else {{
                {S}::__private::register_static(&DESC, &CELL, || {{ {regs} }})
            }}
        }}
    }}
    {asserts}
}};

impl {id} {{
    /// Stable table key (`xxh64` of the table name).
    pub const KEY: {S}::TableKey = <{id} as {S}::NveTable>::DESC.key;
    {props}
}}

/// Zero-copy typed reader of one `{id}` row.
#[derive(Clone, Copy)]
{vis} struct {id}Ref<'a>({S}::RowRef<'a>);

impl<'a> {id}Ref<'a> {{ {getters} }}

/// Fluent typed writer of one `{id}` row.
{vis} struct {id}Mut<'a>({S}::RowWriter<'a>);

impl<'a> {id}Mut<'a> {{
    /// Current row values.
    #[inline(always)]
    pub fn get(&self) -> {id}Ref<'_> {{ {id}Ref(self.0.as_row_ref()) }}
    {setters}
}}
"#)
}
