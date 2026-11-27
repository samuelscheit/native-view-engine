//! Code emission for `#[derive(NveTable)]`.

use crate::emit::quote_str;
use crate::tok::render_expr_tokens;

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
        "NonZeroU8" | "NonZero<u8>" => format!("{S}::__private::opt_nz_u8_to_word({opt_ident})"),
        "NonZeroU16" | "NonZero<u16>" => format!("{S}::__private::opt_nz_u16_to_word({opt_ident})"),
        "NonZeroU32" | "NonZero<u32>" => format!("{S}::__private::opt_nz_u32_to_word({opt_ident})"),
        "NodeId" => format!("{S}::__private::opt_node_to_word({opt_ident})"),
        "SurfaceId" => format!("{S}::__private::opt_surf_to_word({opt_ident})"),
        _ => format!("<{inner_full}>::opt_to_word({opt_ident})"),
    }
}

/// Emits a single-evaluated, concretely typed `const __NVE_DEF_{idx}_{FIELD}: {ty} = ...;` binding
/// (plus a compile-time narrowed `#[bits(N)]` range assertion naming the field) and returns
/// `(default_lo, default_hi, default_present)`.
fn emit_field_default(idx: usize, f: &Field, def_consts: &mut String) -> (String, String, String) {
    let Some(toks) = f.default_toks.as_deref() else {
        return ("0u32".into(), "0u32".into(), "false".into());
    };
    let expr = render_expr_tokens(toks);
    let ty = &f.ty;
    let fname = &f.name;
    let ident = format!("__NVE_DEF_{idx}_{}", upper(fname));
    def_consts.push_str(&format!("const {ident}: {ty} = ({expr});\n"));

    if let Some(n) = f.bits {
        match f.short_ty.as_str() {
            "u8" | "u16" | "u32" => {
                def_consts.push_str(&format!(
                    "const _: () = assert!(({ident} as u64) < (1u64 << {n}), \"default for `{fname}` overflows #[bits({n})] unsigned range\");\n"
                ));
            }
            "i8" | "i16" | "i32" => {
                def_consts.push_str(&format!(
                    "const _: () = assert!(({ident} as i64) >= -(1i64 << ({n} - 1)) && ({ident} as i64) < (1i64 << ({n} - 1)), \"default for `{fname}` overflows #[bits({n})] signed range\");\n"
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

    if f.short_ty == "f32" {
        def_consts.push_str(&format!(
            "const _: () = assert!((f32::to_bits({ident}) & 0x7F80_0000) != 0x7F80_0000, \"default for `{fname}` must be finite (NaN/Inf sentinels are forbidden; use `Option<f32>`)\");\n"
        ));
    } else if f.short_ty == "f64" {
        def_consts.push_str(&format!(
            "const _: () = assert!((f64::to_bits({ident}) & 0x7FF0_0000_0000_0000) != 0x7FF0_0000_0000_0000, \"default for `{fname}` must be finite (NaN/Inf sentinels are forbidden; use `Option<f64>`)\");\n"
        ));
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

    let inherited = t.inherited;
    let (
        mut def_consts,
        mut pack_inputs,
        mut props,
        mut getters,
        mut setters,
        mut regs,
        mut resolve_stmts,
        mut inherit_only_stmts,
    ) = (
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        String::new(),
    );
    let mut inherited_field_mask = 0u32;
    let mut has_no_inherit = false;

    if inherited {
        getters.push_str(
            "#[doc = \"Raw explicit-override bitmask (`word 0`); bit `i` is set iff field `i` was explicitly written.\"] \
            #[inline(always)] pub fn explicit_mask(&self) -> u32 { self.0.word(0) }",
        );
    }

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
        let prop_explicit_suffix = if inherited {
            format!(".with_explicit({idx}u8, __F.effects)")
        } else {
            String::new()
        };
        let set_explicit_stmt = if inherited {
            format!("self.0.set_explicit_bit({idx}u32, __F.effects);")
        } else {
            String::new()
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
                no_inherit: {}, \
            }},",
            quote_str(fname),
            f.effects,
            f.no_inherit,
        ));
        regs.push_str(&format!("<{ty} as {S}::TableField>::register();"));

        if inherited {
            let bit_check = if idx == 0 {
                "(self.0.word(0) & 1u32) != 0".to_owned()
            } else {
                format!("(self.0.word(0) & (1u32 << {idx})) != 0")
            };
            getters.push_str(&format!(
                "#[doc = \"True iff `{fname}` was explicitly written on this row (rather than inherited).\"] \
                #[inline(always)] pub fn is_{fname}_explicit(&self) -> bool {{ {bit_check} }}"
            ));

            if f.no_inherit {
                has_no_inherit = true;
                resolve_stmts.push_str(&format!(
                    "{{ \
                        const __F: {S}::FieldDesc = <{id} as {S}::NveTable>::DESC.fields[{idx}]; \
                        const __W: usize = __F.word as usize; \
                        const __W1: usize = if __F.bits == 64 {{ __F.word as usize + 1 }} else {{ 0 }}; \
                        const __VW: usize = if __F.val_word == {S}::__private::NO_VAL_WORD {{ 0 }} else {{ __F.val_word as usize }}; \
                        const __MASK: u32 = if __F.bits >= 32 {{ u32::MAX }} else {{ ((1u32 << __F.bits) - 1) << __F.shift }}; \
                        if __F.val_word != {S}::__private::NO_VAL_WORD {{ \
                            out[__W] = (out[__W] & !__MASK) | (own[__W] & __MASK); \
                            out[__VW] = own[__VW]; \
                        }} else if __F.bits == 64 {{ \
                            out[__W] = own[__W]; \
                            out[__W1] = own[__W1]; \
                        }} else if __F.bits == 32 {{ \
                            out[__W] = own[__W]; \
                        }} else {{ \
                            out[__W] = (out[__W] & !__MASK) | (own[__W] & __MASK); \
                        }} \
                    }}"
                ));
                inherit_only_stmts.push_str(&format!(
                    "{{ \
                        const __F: {S}::FieldDesc = <{id} as {S}::NveTable>::DESC.fields[{idx}]; \
                        const __W: usize = __F.word as usize; \
                        const __W1: usize = if __F.bits == 64 {{ __F.word as usize + 1 }} else {{ 0 }}; \
                        const __VW: usize = if __F.val_word == {S}::__private::NO_VAL_WORD {{ 0 }} else {{ __F.val_word as usize }}; \
                        const __MASK: u32 = if __F.bits >= 32 {{ u32::MAX }} else {{ ((1u32 << __F.bits) - 1) << __F.shift }}; \
                        if __F.val_word != {S}::__private::NO_VAL_WORD {{ \
                            out[__W] = (out[__W] & !__MASK) | (__NVE_DEFAULTS_ARR[__W] & __MASK); \
                            out[__VW] = __NVE_DEFAULTS_ARR[__VW]; \
                        }} else if __F.bits == 64 {{ \
                            out[__W] = __NVE_DEFAULTS_ARR[__W]; \
                            out[__W1] = __NVE_DEFAULTS_ARR[__W1]; \
                        }} else if __F.bits == 32 {{ \
                            out[__W] = __NVE_DEFAULTS_ARR[__W]; \
                        }} else {{ \
                            out[__W] = (out[__W] & !__MASK) | (__NVE_DEFAULTS_ARR[__W] & __MASK); \
                        }} \
                    }}"
                ));
            } else {
                inherited_field_mask |= 1u32 << idx;
                let mask_bit = if idx == 0 {
                    "__exp & 1".to_owned()
                } else {
                    format!("(__exp >> {idx}) & 1")
                };
                resolve_stmts.push_str(&format!(
                    "{{ \
                        const __F: {S}::FieldDesc = <{id} as {S}::NveTable>::DESC.fields[{idx}]; \
                        const __W: usize = __F.word as usize; \
                        const __W1: usize = if __F.bits == 64 {{ __F.word as usize + 1 }} else {{ 0 }}; \
                        const __VW: usize = if __F.val_word == {S}::__private::NO_VAL_WORD {{ 0 }} else {{ __F.val_word as usize }}; \
                        const __MASK: u32 = if __F.bits >= 32 {{ u32::MAX }} else {{ ((1u32 << __F.bits) - 1) << __F.shift }}; \
                        let __m = 0u32.wrapping_sub({mask_bit}); \
                        if __F.val_word != {S}::__private::NO_VAL_WORD {{ \
                            let __fm = __MASK & __m; \
                            out[__W] = (out[__W] & !__fm) | (own[__W] & __fm); \
                            out[__VW] = (own[__VW] & __m) | (parent[__VW] & !__m); \
                        }} else if __F.bits == 64 {{ \
                            out[__W] = (own[__W] & __m) | (parent[__W] & !__m); \
                            out[__W1] = (own[__W1] & __m) | (parent[__W1] & !__m); \
                        }} else if __F.bits == 32 {{ \
                            out[__W] = (own[__W] & __m) | (parent[__W] & !__m); \
                        }} else {{ \
                            let __fm = __MASK & __m; \
                            out[__W] = (out[__W] & !__fm) | (own[__W] & __fm); \
                        }} \
                    }}"
                ));
            }
        }

        match &f.class {
            Class::Scalar => {
                props.push_str(&format!(
                    "{d} pub const {cname}: {S}::Prop<{id}, {ty}> = {{ \
                        const __F: {S}::FieldDesc = <{id} as {S}::NveTable>::DESC.fields[{idx}]; \
                        unsafe {{ {S}::Prop::new_wide(__F.word, __F.shift, __F.bits, __F.val_word){prop_explicit_suffix} }} \
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
                        {set_explicit_stmt} \
                        self \
                    }}"
                ));
                if inherited {
                    setters.push_str(&format!(
                        "#[doc = \"Clears explicit `{fname}`, resetting it to default so it inherits from its parent.\"] \
                        #[inline(always)] pub fn clear_{fname}(mut self) -> Self {{ \
                            const __F: {S}::FieldDesc = <{id} as {S}::NveTable>::DESC.fields[{idx}]; \
                            const __MASK: u32 = if __F.bits >= 32 {{ u32::MAX }} else {{ ((1u32 << __F.bits) - 1) << __F.shift }}; \
                            let __def = <{ty} as {S}::ScalarField>::read_row( \
                                <{id} as {S}::NveTable>::DESC.defaults, \
                                __F.word as usize, \
                                __F.shift as u32, \
                                __MASK, \
                                __F.val_word as usize, \
                            ); \
                            <{ty} as {S}::ScalarField>::write_row(__def, &mut self.0, __F.word as usize, __F.shift as u32, __MASK, __F.val_word as usize); \
                            self.0.clear_explicit_bit({idx}u32, __F.effects); \
                            self \
                        }}"
                    ));
                }
            }
            Class::Str => {
                props.push_str(&format!(
                    "{d} pub const {cname}: {S}::StrProp<{id}> = {{ \
                        const __F: {S}::FieldDesc = <{id} as {S}::NveTable>::DESC.fields[{idx}]; \
                        unsafe {{ {S}::StrProp::new(__F.word){prop_explicit_suffix} }} \
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
                        const __F: {S}::FieldDesc = <{id} as {S}::NveTable>::DESC.fields[{idx}]; \
                        self.0.put_str(__F.word as usize, v); \
                        {set_explicit_stmt} \
                        self \
                    }}"
                ));
                if inherited {
                    setters.push_str(&format!(
                        "#[doc = \"Clears explicit `{fname}`, resetting it to empty so it inherits from its parent.\"] \
                        #[inline] pub fn clear_{fname}(mut self) -> Self {{ \
                            const __F: {S}::FieldDesc = <{id} as {S}::NveTable>::DESC.fields[{idx}]; \
                            self.0.put_str(__F.word as usize, \"\"); \
                            self.0.clear_explicit_bit({idx}u32, __F.effects); \
                            self \
                        }}"
                    ));
                }
            }
            Class::Res(r) => {
                props.push_str(&format!(
                    "{d} pub const {cname}: {S}::ResProp<{id}, {r}> = {{ \
                        const __F: {S}::FieldDesc = <{id} as {S}::NveTable>::DESC.fields[{idx}]; \
                        unsafe {{ {S}::ResProp::new(__F.word){prop_explicit_suffix} }} \
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
                        const __F: {S}::FieldDesc = <{id} as {S}::NveTable>::DESC.fields[{idx}]; \
                        self.0.put_resource(__F.word as usize, v); \
                        {set_explicit_stmt} \
                        self \
                    }} \
                    #[doc = \"Ephemeral (in-place, non-deduplicated) write; for per-frame blobs.\"] \
                    #[inline] pub fn {fname}_ephemeral(mut self, v: {r}) -> Self {{ \
                        const __F: {S}::FieldDesc = <{id} as {S}::NveTable>::DESC.fields[{idx}]; \
                        self.0.put_resource_ephemeral(__F.word as usize, v); \
                        {set_explicit_stmt} \
                        self \
                    }}"
                ));
                if inherited {
                    setters.push_str(&format!(
                        "#[doc = \"Clears explicit `{fname}`, releasing the resource so it inherits from its parent.\"] \
                        #[inline] pub fn clear_{fname}(mut self) -> Self {{ \
                            const __F: {S}::FieldDesc = <{id} as {S}::NveTable>::DESC.fields[{idx}]; \
                            self.0.put_resource::<{r}>(__F.word as usize, ::core::option::Option::None); \
                            self.0.clear_explicit_bit({idx}u32, __F.effects); \
                            self \
                        }}"
                    ));
                }
            }
        }
    }

    let inherited_impl = if inherited {
        let inherit_only_body = if has_no_inherit {
            format!(
                "let mut out = *parent; \
                 out[0] = parent[0] & Self::INHERITED_FIELD_MASK; \
                 {inherit_only_stmts} \
                 out"
            )
        } else {
            "*parent".to_owned()
        };
        format!(
            r#"
    unsafe impl {S}::InheritedTable for {id} {{
        type Words = [{S}::Word; __NVE_WORD_COUNT];
        const INHERITED_FIELD_MASK: u32 = {inherited_field_mask:#010x}u32;

        #[inline(always)]
        fn default_words() -> Self::Words {{
            __NVE_DEFAULTS_ARR
        }}

        #[inline(always)]
        fn inherit_only(parent: &Self::Words) -> Self::Words {{
            {inherit_only_body}
        }}

        #[inline]
        fn resolve_inherited_arrays(own: &Self::Words, parent: &Self::Words) -> Self::Words {{
            let __exp = own[0];
            let mut out = *parent;
            out[0] = __exp | (parent[0] & Self::INHERITED_FIELD_MASK);
            {resolve_stmts}
            out
        }}

        #[inline]
        fn resolve_inherited_words(own: &[{S}::Word], parent: &Self::Words) -> ::core::option::Option<Self::Words> {{
            let __own = own.get(..__NVE_WORD_COUNT).and_then(|s| <&[{S}::Word; __NVE_WORD_COUNT]>::try_from(s).ok())?;
            ::core::option::Option::Some(Self::resolve_inherited_arrays(__own, parent))
        }}
    }}
"#
        )
    } else {
        String::new()
    };

    format!(
        r#"
const _: () = {{
    const __NVE_LAYOUT: {S}::__private::PackLayout<{field_count}> = {{
        {def_consts}
        {S}::__private::pack_table(
            [{pack_inputs}],
            {inherited},
            false,
        )
    }};
    const __NVE_WORD_COUNT: usize = __NVE_LAYOUT.word_count;
    const __NVE_DEFAULTS_ARR: [{S}::Word; __NVE_WORD_COUNT] = {S}::__private::trim_words::<__NVE_WORD_COUNT>(&__NVE_LAYOUT.defaults);
    const __NVE_EFFECTS_ARR: [u8; __NVE_WORD_COUNT] = {S}::__private::trim_bytes::<__NVE_WORD_COUNT>(&__NVE_LAYOUT.word_effects);
    const __NVE_FINAL_FLAGS: u8 = {S}::__private::finalize_table_flags({base_flags}, &__NVE_EFFECTS_ARR);
    const __NVE_FIELDS_ARR: [{S}::FieldDesc; {field_count}] = __NVE_LAYOUT.fields;

    unsafe impl {S}::NveTable for {id} {{
        const DESC: {S}::TableDesc = {S}::TableDesc {{
            key: {S}::__private::Key64::of({name}),
            name: {name},
            schema_hash: {S}::__private::compute_table_schema_hash(
                {name},
                {S}::Storage::{storage},
                __NVE_FINAL_FLAGS,
                &__NVE_DEFAULTS_ARR,
                &__NVE_EFFECTS_ARR,
                __NVE_LAYOUT.str_words,
                __NVE_LAYOUT.resource_words,
                __NVE_LAYOUT.events_word,
                {inherited},
                &__NVE_FIELDS_ARR,
            ),
            storage: {S}::Storage::{storage},
            flags: __NVE_FINAL_FLAGS,
            defaults: &__NVE_DEFAULTS_ARR,
            word_effects: &__NVE_EFFECTS_ARR,
            str_words: __NVE_LAYOUT.str_words,
            resource_words: __NVE_LAYOUT.resource_words,
            events_word: __NVE_LAYOUT.events_word,
            inherited: {inherited},
            fields: &__NVE_FIELDS_ARR,
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
    {inherited_impl}

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
