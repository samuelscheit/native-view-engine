//! Code emission for `#[derive(NveTable)]`.

use crate::emit::quote_str;
use crate::tok::render_tokens;

use super::{upper, Class, Field, Table, S};

/// Encodes an already type-checked local/const identifier `val` of declared scalar type `short_ty`
/// into `(default_lo, default_hi)` raw words.
fn encode_typed_scalar(short_ty: &str, val: &str) -> (String, String) {
    match short_ty {
        "f32" => (format!("f32::to_bits({val})"), "0u32".into()),
        "f64" => (
            format!("(f64::to_bits({val}) as u32)"),
            format!("((f64::to_bits({val}) >> 32) as u32)"),
        ),
        "u64" => (format!("({val} as u32)"), format!("(({val} >> 32) as u32)")),
        "i64" => (
            format!("(({val} as u64) as u32)"),
            format!("((({val} as u64) >> 32) as u32)"),
        ),
        "u32" | "u16" | "u8" | "i32" | "bool" => (format!("({val} as u32)"), "0u32".into()),
        "i16" => (format!("(({val} as u16) as u32)"), "0u32".into()),
        "i8" => (format!("(({val} as u8) as u32)"), "0u32".into()),
        "NodeId" | "SurfaceId" => (format!("{val}.raw()"), "0u32".into()),
        _ => (format!("{val}.to_word()"), "0u32".into()),
    }
}

fn is_wide_opt_inner(inner_short: &str) -> bool {
    matches!(inner_short, "f32" | "u32" | "i32" | "Rgba")
}

fn narrow_opt_default_expr(inner_short: &str, inner_full: &str, opt_ident: &str) -> String {
    match inner_short {
        "bool" => format!("{S}::__private::opt_bool_to_word({opt_ident})"),
        "u8" => format!("{S}::__private::opt_u8_to_word({opt_ident})"),
        "i8" => format!("{S}::__private::opt_i8_to_word({opt_ident})"),
        "u16" => format!("{S}::__private::opt_u16_to_word({opt_ident})"),
        "i16" => format!("{S}::__private::opt_i16_to_word({opt_ident})"),
        "NodeId" => format!("{S}::__private::opt_node_to_word({opt_ident})"),
        "SurfaceId" => format!("{S}::__private::opt_surf_to_word({opt_ident})"),
        _ => format!("<{inner_full}>::opt_to_word({opt_ident})"),
    }
}

/// Emits a single-evaluated, concretely typed `const __NVE_DEF_{idx}: {ty} = ...;` binding (plus
/// a compile-time narrowed `#[bits(N)]` range assertion when applicable) and returns
/// `(default_lo, default_hi, default_present)`.
fn emit_field_default(idx: usize, f: &Field, def_consts: &mut String) -> (String, String, String) {
    let Some(toks) = f.default_toks.as_deref() else {
        return ("0u32".into(), "0u32".into(), "false".into());
    };
    let expr = render_tokens(toks);
    let ty = &f.ty;
    let ident = format!("__NVE_DEF_{idx}");
    def_consts.push_str(&format!("const {ident}: {ty} = ({expr});\n"));

    if let Some(n) = f.bits {
        match f.short_ty.as_str() {
            "u8" | "u16" | "u32" => {
                def_consts.push_str(&format!(
                    "const _: () = assert!(({ident} as u64) < (1u64 << {n}), \"default value overflows #[bits({n})] unsigned range\");\n"
                ));
            }
            "i8" | "i16" | "i32" => {
                def_consts.push_str(&format!(
                    "const _: () = assert!(({ident} as i64) >= -(1i64 << ({n} - 1)) && ({ident} as i64) < (1i64 << ({n} - 1)), \"default value overflows #[bits({n})] signed range\");\n"
                ));
            }
            _ => {}
        }
    }

    if let (Some(inner_short), Some(inner_full)) =
        (f.inner_opt_short.as_deref(), f.inner_opt_full.as_deref())
    {
        if is_wide_opt_inner(inner_short) {
            let (some_lo, _) = encode_typed_scalar(inner_short, "__nve_v");
            let lo = format!(
                "match {ident} {{ ::core::option::Option::Some(__nve_v) => {some_lo}, ::core::option::Option::None => 0u32 }}"
            );
            let present = format!("::core::option::Option::is_some(&{ident})");
            return (lo, "0u32".into(), present);
        }
        return (
            narrow_opt_default_expr(inner_short, inner_full, &ident),
            "0u32".into(),
            "false".into(),
        );
    }

    let (lo, hi) = encode_typed_scalar(&f.short_ty, &ident);
    (lo, hi, "false".into())
}

fn docs(f: &Field) -> String {
    f.doc
        .iter()
        .map(|d| format!("#[doc = {}]", quote_str(d)))
        .collect()
}

pub(super) fn emit(t: &Table) -> String {
    let id = &t.ident;
    let vis = &t.vis;
    let field_count = t.fields.len();
    let base_flags = t.flags;
    let storage = if t.dense { "Dense" } else { "Sparse" };
    let name = quote_str(&t.name);

    let (mut def_consts, mut pack_inputs, mut props, mut getters, mut setters, mut regs) = (
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
        let (def_lo, def_hi, def_present) = emit_field_default(idx, f, &mut def_consts);
        let bits_expr = match f.bits {
            Some(b) => format!("{b}u8"),
            None => format!("<{ty} as {S}::TableField>::BITS"),
        };

        pack_inputs.push_str(&format!(
            "{S}::__private::PackInput {{ \
                name: {}, \
                field_kind: <{ty} as {S}::TableField>::KIND, \
                opt_encoding: <{ty} as {S}::TableField>::OPT_ENCODING, \
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
    {def_consts}
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

    // Force eager const evaluation of `pack_table` & `DESC` at every derive site.
    let _ = <{id} as {S}::NveTable>::DESC;
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
