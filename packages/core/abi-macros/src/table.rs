//! `#[derive(NveTable)]`: word-table schema → zero-cost typed rows for `nve-store`.
//!
//! Uses compile-time `nve_abi::table::pack_table` first-fit bin-packing over effect-matched
//! words, supporting narrow scalars (`WordValue`), narrow `Option<T: OptWord>`, wide
//! `Option<f32|u32|i32|Rgba>`, 64-bit scalars, `Str`, and `Res<R>`.

use proc_macro::TokenStream;

use crate::emit::quote_str;
use crate::parse::{parse_string_literal, parse_usize_literal};
use crate::tok::{
    expect_ident, from_stream, is_ident, is_punct, render_tokens, skip_vis, Delim, Tok,
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
    inner_opt_ty: Option<String>,
    class: Class,
    effects: u8,
    default: Option<String>,
    bits: Option<u32>,
    doc: Vec<String>,
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
    let tokens = from_stream(input);
    derive_tokens(&tokens)
}

pub(crate) fn derive_tokens(tokens: &[Tok]) -> Result<String, String> {
    let t = parse(tokens)?;
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
        other => {
            return Err(format!(
                "unknown effect `{other}` (layout|measure|paint|composite|semantics|realize|events)"
            ))
        }
    })
}

fn paren_inner(rest: &[Tok]) -> Option<&[Tok]> {
    match rest.first() {
        Some(Tok::Group(Delim::Paren, inner)) => Some(inner.as_slice()),
        _ => None,
    }
}

fn parse(tokens: &[Tok]) -> Result<Table, String> {
    let mut pos = 0;
    let (mut name, mut dense, mut flags) = (None, true, 0u8);
    while is_punct(tokens, pos, '#') {
        if let Some(Tok::Group(Delim::Bracket, inner)) = tokens.get(pos + 1)
            && matches!(inner.first(), Some(Tok::Ident(i)) if i == "nve_table")
        {
            let kv = paren_inner(&inner[1..]).ok_or("expected `#[nve_table(...)]`")?;
            let mut i = 0;
            while i < kv.len() {
                let Tok::Ident(k) = &kv[i] else {
                    return Err(format!("expected key identifier in #[nve_table], found `{:?}`", kv[i]));
                };
                if !is_punct(kv, i + 1, '=') {
                    return Err(format!("expected `=` after #[nve_table] key `{k}`"));
                }
                let Tok::Literal(v) = kv.get(i + 2).ok_or("expected value in #[nve_table]")? else {
                    return Err(format!("expected string literal for #[nve_table] key `{k}`"));
                };
                let v = parse_string_literal(v)?;
                match k.as_str() {
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
                    if !is_punct(kv, i, ',') {
                        return Err(format!("expected `,` in #[nve_table], found `{:?}`", kv[i]));
                    }
                    i += 1;
                }
            }
        }
        pos += 2;
    }

    let vis_start = pos;
    skip_vis(tokens, &mut pos);
    let vis = render_tokens(&tokens[vis_start..pos]);
    let kw = expect_ident(tokens, &mut pos, "struct")?;
    if kw != "struct" {
        return Err("#[derive(NveTable)] only supports structs".into());
    }
    let ident = expect_ident(tokens, &mut pos, "struct name")?;
    if is_punct(tokens, pos, '<') || is_ident(tokens, pos, "where") {
        return Err(format!("#[derive(NveTable)] does not support generics on `{ident}`"));
    }
    let Some(Tok::Group(Delim::Brace, body)) = tokens.get(pos) else {
        return Err(format!("#[derive(NveTable)] requires named fields on `{ident}`"));
    };
    let name = name.ok_or_else(|| format!("`{ident}` needs #[nve_table(name = \"...\")]"))?;
    let fields = parse_fields(body)?;
    if fields.is_empty() {
        return Err(format!("table `{ident}` must declare at least 1 field"));
    }
    Ok(Table { vis, ident, name, dense, flags, fields })
}

fn parse_fields(tokens: &[Tok]) -> Result<Vec<Field>, String> {
    let (mut pos, mut out) = (0, Vec::new());
    while pos < tokens.len() {
        let (mut effects, mut default, mut bits, mut doc) = (0u8, None, None, Vec::new());
        while is_punct(tokens, pos, '#') {
            let Some(Tok::Group(Delim::Bracket, inner)) = tokens.get(pos + 1) else {
                return Err("bad attribute".into());
            };
            let attr = match inner.first() {
                Some(Tok::Ident(i)) => i.as_str(),
                _ => "",
            };
            match attr {
                "effect" => {
                    for t in paren_inner(&inner[1..]).ok_or("expected `#[effect(...)]`")? {
                        if let Tok::Ident(e) = t {
                            effects |= effect_bit(e)?;
                        }
                    }
                }
                "default" => {
                    let g = paren_inner(&inner[1..]).ok_or("expected `#[default(expr)]`")?;
                    default = Some(render_tokens(g));
                }
                "bits" => {
                    let g = paren_inner(&inner[1..]).ok_or("expected `#[bits(n)]`")?;
                    let [Tok::Literal(l)] = g else {
                        return Err("expected integer in #[bits]".into());
                    };
                    let n = parse_usize_literal(l)? as u32;
                    if n == 0 || n > 32 {
                        return Err(format!("#[bits({n})] must be 1..=32"));
                    }
                    bits = Some(n);
                }
                "doc" => {
                    if let Some(Tok::Literal(l)) = inner.get(2) {
                        doc.push(parse_string_literal(l)?);
                    }
                }
                _ => {}
            }
            pos += 2;
        }
        if pos >= tokens.len() {
            break;
        }
        skip_vis(tokens, &mut pos);
        let name = expect_ident(tokens, &mut pos, "field name")?;
        if !is_punct(tokens, pos, ':') {
            return Err(format!("expected `:` after `{name}`"));
        }
        pos += 1;
        let start = pos;
        let mut depth = 0i32;
        while pos < tokens.len() {
            match &tokens[pos] {
                Tok::Punct('<', _) => depth += 1,
                Tok::Punct('>', _) => depth -= 1,
                Tok::Punct(',', _) if depth == 0 => break,
                _ => {}
            }
            pos += 1;
        }
        let ty = render_tokens(&tokens[start..pos]);
        if is_punct(tokens, pos, ',') {
            pos += 1;
        }
        let compact: String = ty.chars().filter(|c| !c.is_whitespace()).collect();
        let outer_head = compact.split('<').next().unwrap_or(&compact);
        let outer_last = outer_head.rsplit("::").next().unwrap_or(outer_head).to_owned();
        let inner_opt_ty = if outer_last == "Option"
            && let Some(lt) = compact.find('<')
            && compact.ends_with('>')
        {
            let inner = &compact[lt + 1..compact.len() - 1];
            Some(inner.rsplit("::").next().unwrap_or(inner).to_owned())
        } else {
            None
        };
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
        if effects == 0 {
            return Err(format!("field `{name}` needs #[effect(...)]"));
        }
        if !matches!(class, Class::Scalar) && (default.is_some() || bits.is_some()) {
            return Err(format!("`{name}`: Str/Res fields take no #[default]/#[bits]"));
        }
        out.push(Field {
            name,
            ty,
            short_ty: outer_last,
            inner_opt_ty,
            class,
            effects,
            default,
            bits,
            doc,
        });
    }
    Ok(out)
}

fn upper(s: &str) -> String {
    s.to_ascii_uppercase()
}

fn scalar_default_expr(short_ty: &str, expr: &str) -> (String, String) {
    match short_ty {
        "f32" => (format!("f32::to_bits({expr})"), "0u32".into()),
        "f64" => (
            format!("(f64::to_bits({expr}) as u32)"),
            format!("((f64::to_bits({expr}) >> 32) as u32)"),
        ),
        "u64" => (
            format!("(({expr}) as u64 as u32)"),
            format!("(((({expr}) as u64) >> 32) as u32)"),
        ),
        "i64" => (
            format!("((({expr}) as i64) as u64 as u32)"),
            format!("((((({expr}) as i64) as u64) >> 32) as u32)"),
        ),
        "u32" | "u16" | "u8" | "i32" | "bool" => (format!("(({expr}) as u32)"), "0u32".into()),
        "i16" => (format!("((({expr}) as i16) as u16 as u32)"), "0u32".into()),
        "i8" => (format!("((({expr}) as i8) as u8 as u32)"), "0u32".into()),
        "NodeId" | "ResourceId" | "SurfaceId" => (format!("(({expr}).raw())"), "0u32".into()),
        _ => (format!("(({expr}).to_word())"), "0u32".into()),
    }
}

fn default_pack_fields(f: &Field) -> (String, String, &'static str) {
    let Some(expr) = f.default.as_deref() else {
        return ("0u32".into(), "0u32".into(), "false");
    };
    let trimmed = expr.trim();
    if let Some(inner_ty) = f.inner_opt_ty.as_deref() {
        if trimmed == "None" || trimmed == "::core::option::Option::None" {
            return ("0u32".into(), "0u32".into(), "false");
        }
        if let Some(inner_expr) = trimmed
            .strip_prefix("Some(")
            .or_else(|| trimmed.strip_prefix("::core::option::Option::Some("))
            .and_then(|s| s.strip_suffix(')'))
        {
            let (lo, hi) = scalar_default_expr(inner_ty, inner_expr);
            return (lo, hi, "true");
        }
    }
    let (lo, hi) = scalar_default_expr(&f.short_ty, trimmed);
    (lo, hi, "false")
}

fn docs(f: &Field) -> String {
    f.doc.iter().map(|d| format!("#[doc = {}]", quote_str(d))).collect()
}

fn emit(t: &Table) -> String {
    let id = &t.ident;
    let vis = &t.vis;
    let field_count = t.fields.len();
    let base_flags = t.flags;
    let storage = if t.dense { "Dense" } else { "Sparse" };
    let name = quote_str(&t.name);

    let (mut pack_inputs, mut props, mut getters, mut setters, mut regs, mut asserts) = (
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        String::new(),
    );

    for (idx, f) in t.fields.iter().enumerate() {
        let ty = &f.ty;
        let fname = &f.name;
        let d = docs(f);
        let cname = upper(fname);
        let (def_lo, def_hi, def_present) = default_pack_fields(f);
        let bits_expr = match f.bits {
            Some(b) => format!("{b}u8"),
            None => format!("<{ty} as {S}::TableField>::BITS"),
        };

        pack_inputs.push_str(&format!(
            "{S}::__private::PackInput {{ \
                name: {}, \
                field_kind: <{ty} as {S}::TableField>::KIND, \
                pack_kind: <{ty} as {S}::TableField>::PACK_KIND, \
                bits: {bits_expr}, \
                effects: {}, \
                default_lo: {def_lo}, \
                default_hi: {def_hi}, \
                default_present: {def_present}, \
            }},",
            quote_str(fname),
            f.effects,
        ));
        regs.push_str(&format!("<{ty} as {S}::TableField>::register();"));

        match &f.class {
            Class::Scalar => {
                if let Some(b) = f.bits {
                    let is_prim_int = matches!(
                        f.short_ty.as_str(),
                        "u8" | "i8" | "u16" | "i16" | "u32" | "i32"
                    );
                    if !is_prim_int {
                        asserts.push_str(&format!(
                            "const _: () = assert!((<{ty} as {S}::TableField>::BITS as u32) <= {b}, \"#[bits] narrower than the field type\");"
                        ));
                    }
                }
                props.push_str(&format!(
                    "{d} pub const {cname}: {S}::Prop<{id}, {ty}> = {{ \
                        const __F: {S}::FieldDesc = <{id} as {S}::NveTable>::DESC.fields[{idx}]; \
                        unsafe {{ {S}::Prop::new_wide(__F.word, __F.shift, __F.bits, __F.val_word) }} \
                    }};"
                ));
                getters.push_str(&format!(
                    "{d} #[inline(always)] pub fn {fname}(&self) -> {ty} {{ \
                        const __F: {S}::FieldDesc = <{id} as {S}::NveTable>::DESC.fields[{idx}]; \
                        const __MASK: u32 = if __F.bits >= 32 {{ u32::MAX }} else {{ ((1u32 << __F.bits) - 1) << __F.shift }}; \
                        <{ty} as {S}::ScalarField>::read_ref(self.0, __F.word as usize, __F.shift as u32, __MASK, __F.val_word as usize) \
                    }}"
                ));
                setters.push_str(&format!(
                    "{d} #[inline(always)] pub fn {fname}(mut self, v: {ty}) -> Self {{ \
                        const __F: {S}::FieldDesc = <{id} as {S}::NveTable>::DESC.fields[{idx}]; \
                        const __MASK: u32 = if __F.bits >= 32 {{ u32::MAX }} else {{ ((1u32 << __F.bits) - 1) << __F.shift }}; \
                        <{ty} as {S}::ScalarField>::write_row(v, &mut self.0, __F.word as usize, __F.shift as u32, __MASK, __F.val_word as usize); \
                        self \
                    }}"
                ));
            }
            Class::Str => {
                props.push_str(&format!(
                    "{d} pub const {cname}: {S}::StrProp<{id}> = {{ \
                        const __F: {S}::FieldDesc = <{id} as {S}::NveTable>::DESC.fields[{idx}]; \
                        unsafe {{ {S}::StrProp::new(__F.word) }} \
                    }};"
                ));
                getters.push_str(&format!(
                    "{d} #[inline(always)] pub fn {fname}(&self) -> &'a str {{ \
                        const __W: usize = <{id} as {S}::NveTable>::DESC.fields[{idx}].word as usize; \
                        self.0.str_at(__W) \
                    }} \
                    #[doc = \"Raw `(off, len)` span into the store text slab.\"] \
                    #[inline(always)] pub fn {fname}_span(&self) -> {S}::StrSpan {{ \
                        const __W: usize = <{id} as {S}::NveTable>::DESC.fields[{idx}].word as usize; \
                        self.0.str_span_at(__W) \
                    }}"
                ));
                setters.push_str(&format!(
                    "{d} #[inline] pub fn {fname}(mut self, v: &str) -> Self {{ \
                        const __W: usize = <{id} as {S}::NveTable>::DESC.fields[{idx}].word as usize; \
                        self.0.put_str(__W, v); \
                        self \
                    }}"
                ));
            }
            Class::Res(r) => {
                props.push_str(&format!(
                    "{d} pub const {cname}: {S}::ResProp<{id}, {r}> = {{ \
                        const __F: {S}::FieldDesc = <{id} as {S}::NveTable>::DESC.fields[{idx}]; \
                        unsafe {{ {S}::ResProp::new(__F.word) }} \
                    }};"
                ));
                getters.push_str(&format!(
                    "{d} #[inline(always)] pub fn {fname}(&self) -> {S}::ResourceId {{ \
                        const __W: usize = <{id} as {S}::NveTable>::DESC.fields[{idx}].word as usize; \
                        {S}::ResourceId(self.0.word(__W)) \
                    }}"
                ));
                setters.push_str(&format!(
                    "{d} #[inline] pub fn {fname}(mut self, v: ::core::option::Option<{r}>) -> Self {{ \
                        const __W: usize = <{id} as {S}::NveTable>::DESC.fields[{idx}].word as usize; \
                        self.0.put_resource(__W, v); \
                        self \
                    }} \
                    #[doc = \"Ephemeral (in-place, non-deduplicated) write; for per-frame blobs.\"] \
                    #[inline] pub fn {fname}_ephemeral(mut self, v: {r}) -> Self {{ \
                        const __W: usize = <{id} as {S}::NveTable>::DESC.fields[{idx}].word as usize; \
                        self.0.put_resource_ephemeral(__W, v); \
                        self \
                    }}"
                ));
            }
        }
    }

    format!(
        r#"
const _: () = {{
    const LAYOUT: {S}::__private::PackLayout<{field_count}> = {S}::__private::pack_table(
        [{pack_inputs}],
        false,
        false,
    );
    const WORD_COUNT: usize = LAYOUT.word_count;
    const DEFAULTS_ARR: [{S}::Word; WORD_COUNT] = {S}::__private::trim_words::<WORD_COUNT>(&LAYOUT.defaults);
    const EFFECTS_ARR: [u8; WORD_COUNT] = {S}::__private::trim_bytes::<WORD_COUNT>(&LAYOUT.word_effects);
    const FINAL_FLAGS: u8 = {S}::__private::finalize_table_flags({base_flags}, &EFFECTS_ARR);
    const FIELDS_ARR: [{S}::FieldDesc; {field_count}] = LAYOUT.fields;

    unsafe impl {S}::NveTable for {id} {{
        const DESC: {S}::TableDesc = {S}::TableDesc {{
            key: {S}::__private::Key64::of({name}),
            name: {name},
            schema_hash: {S}::__private::compute_table_schema_hash(
                {name},
                {S}::Storage::{storage},
                FINAL_FLAGS,
                &DEFAULTS_ARR,
                &EFFECTS_ARR,
                LAYOUT.str_words,
                LAYOUT.resource_words,
                LAYOUT.events_word,
                false,
                &FIELDS_ARR,
            ),
            storage: {S}::Storage::{storage},
            flags: FINAL_FLAGS,
            defaults: &DEFAULTS_ARR,
            word_effects: &EFFECTS_ARR,
            str_words: LAYOUT.str_words,
            resource_words: LAYOUT.resource_words,
            events_word: LAYOUT.events_word,
            inherited: false,
            fields: &FIELDS_ARR,
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
"#
    )
}
