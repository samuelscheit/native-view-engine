//! Shared Kotlin binary `NveBinValue` (`BinWriter` / `BinReader`) and React Native Android
//! bridge (`ReadableArray` / `ReadableMap` / `WritableArray` / `WritableMap`) codec emitters used
//! by both `compose.rs` (`export_compose`) and `export_rn_android.rs` (`export_rn`).
//!
//! All generated internal locals and loop indices are `_`-prefixed (`_w`, `_r`, `_mark`, `_count`,
//! `_arr0`, `_list0`, `_m0`, `_i0`, `_j0`, ...) so they can never shadow user identifiers produced
//! by `kotlin_ident` (which always start with an ASCII letter or backtick).

use crate::naming::{escape_kotlin_string, format_float_literal};
use crate::schema::export::{DefaultValue, NveExportSpec, ObjectField, PropSpec, PropType};
use crate::writer::CodeWriter;

pub(super) fn uses_dynamic_value(ty: &PropType) -> bool {
    match ty {
        PropType::Object { .. } => true,
        PropType::Array { item } => uses_dynamic_value(item),
        _ => false,
    }
}

pub(super) fn spec_uses_dynamic_value(spec: &NveExportSpec) -> bool {
    spec.props.iter().any(|p| uses_dynamic_value(&p.prop_type))
        || spec
            .events
            .iter()
            .any(|e| e.payload.iter().any(|f| uses_dynamic_value(&f.field_type)))
        || spec
            .commands
            .iter()
            .any(|c| c.params.iter().any(|p| uses_dynamic_value(&p.param_type)))
}

pub(super) fn type_uses_rn_map(ty: &PropType) -> bool {
    match ty {
        PropType::Object { .. } | PropType::Point | PropType::EdgeInsets => true,
        PropType::Array { item } => type_uses_rn_map(item),
        _ => false,
    }
}

pub(super) fn spec_uses_rn_readable_map(spec: &NveExportSpec) -> bool {
    spec.props.iter().any(|p| type_uses_rn_map(&p.prop_type))
        || spec
            .commands
            .iter()
            .any(|c| c.params.iter().any(|p| type_uses_rn_map(&p.param_type)))
}

pub(super) fn spec_uses_rn_readable_array(spec: &NveExportSpec) -> bool {
    !spec.commands.is_empty()
        || spec.props.iter().any(|p| {
            matches!(p.prop_type, PropType::Array { .. }) || type_uses_rn_map(&p.prop_type)
        })
}

pub(super) fn type_uses_rn_writable_containers(ty: &PropType) -> bool {
    matches!(
        ty,
        PropType::Array { .. }
            | PropType::Object { .. }
            | PropType::Point
            | PropType::EdgeInsets
            | PropType::Mixed
    )
}

pub(super) fn spec_uses_rn_writable_helpers(spec: &NveExportSpec) -> bool {
    spec.events.iter().any(|e| {
        e.payload
            .iter()
            .any(|f| type_uses_rn_writable_containers(&f.field_type))
    })
}

pub(super) fn kotlin_simple_type(ty: &PropType) -> String {
    match ty {
        PropType::Bool => "Boolean".to_owned(),
        PropType::Int32 | PropType::IntEnum { .. } => "Int".to_owned(),
        PropType::Float => "Float".to_owned(),
        PropType::Double => "Double".to_owned(),
        PropType::Array { item } => format!("List<{}>", kotlin_simple_type(item)),
        PropType::Object { .. } => "HybridRuntime.DynamicValue".to_owned(),
        _ => "String".to_owned(),
    }
}

pub(super) fn kotlin_zero_default(ty: &PropType) -> String {
    match ty {
        PropType::Bool => "false".to_owned(),
        PropType::Int32 | PropType::IntEnum { .. } => "0".to_owned(),
        PropType::Float => "0.0f".to_owned(),
        PropType::Double => "0.0".to_owned(),
        PropType::Array { .. } => "emptyList()".to_owned(),
        PropType::Object { .. } => "HybridRuntime.DynamicValue.NULL".to_owned(),
        _ => "\"\"".to_owned(),
    }
}

/// Emits recursive Kotlin statements encoding `var` into `BinWriter` `w` (used by Compose props
/// and imperative commands). All generated locals are `_`-prefixed.
pub(super) fn emit_compose_bin_write_value(
    out: &mut CodeWriter,
    indent: &str,
    w: &str,
    var: &str,
    ty: &PropType,
    depth: usize,
) {
    match ty {
        PropType::Bool => writeln!(out, "{indent}{w}.writeBool({var})"),
        PropType::Int32 | PropType::IntEnum { .. } => {
            writeln!(out, "{indent}{w}.writeI64({var}.toLong())")
        }
        PropType::Float => writeln!(out, "{indent}{w}.writeF64({var}.toDouble())"),
        PropType::Double => writeln!(out, "{indent}{w}.writeF64({var})"),
        PropType::Array { item } => {
            let mark = format!("_arrMark{depth}");
            let elem = format!("_item{depth}");
            let inner = format!("{indent}    ");
            writeln!(out, "{indent}val {mark} = {w}.beginArray({var}.size)");
            writeln!(out, "{indent}for ({elem} in {var}) {{");
            emit_compose_bin_write_value(out, &inner, w, &elem, item, depth + 1);
            writeln!(out, "{indent}}}");
            writeln!(out, "{indent}{w}.end({mark})");
        }
        PropType::Object { .. } => writeln!(out, "{indent}{var}.writeTo({w})"),
        _ => writeln!(out, "{indent}{w}.writeStr({var})"),
    }
}

/// Emits recursive Kotlin statements decoding one value from `BinReader` `r` into either a local
/// variable assignment (`is_list_add == false`) or `ArrayList::add` (`is_list_add == true`).
/// Handles nested `Array<Array<T>>`, `Array<Object>`, `Color` (`STR` or masked `I64`), and
/// nullable `Object?` (`BinTag.NULL` -> Kotlin `null`).
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_compose_bin_read_value(
    out: &mut CodeWriter,
    indent: &str,
    r: &str,
    target: &str,
    is_list_add: bool,
    ty: &PropType,
    nullable: bool,
    depth: usize,
) {
    if nullable && !is_list_add {
        let inner = format!("{indent}    ");
        writeln!(out, "{indent}if ({r}.peekTag() == BinTag.NULL) {{");
        writeln!(out, "{inner}{r}.readNull()");
        writeln!(out, "{inner}{target} = null");
        writeln!(out, "{indent}}} else {{");
        emit_compose_bin_read_value(out, &inner, r, target, false, ty, false, depth);
        writeln!(out, "{indent}}}");
        return;
    }

    let assign = |expr: &str| {
        if is_list_add {
            format!("{target}.add({expr})")
        } else {
            format!("{target} = {expr}")
        }
    };

    match ty {
        PropType::Bool => {
            let a = assign(&format!("{r}.readBool()"));
            writeln!(
                out,
                "{indent}if ({r}.peekTag() == BinTag.BOOL) {a} else {r}.skipValue()"
            );
        }
        PropType::Int32 | PropType::IntEnum { .. } => {
            let a_i = assign(&format!("{r}.readI64().toInt()"));
            let a_f = assign(&format!("{r}.readF64().toInt()"));
            writeln!(
                out,
                "{indent}when ({r}.peekTag()) {{ BinTag.I64 -> {a_i}; BinTag.F64 -> {a_f}; else -> {r}.skipValue() }}"
            );
        }
        PropType::Float => {
            let a = assign(&format!("{r}.readF64().toFloat()"));
            writeln!(
                out,
                "{indent}if ({r}.peekTag() == BinTag.F64 || {r}.peekTag() == BinTag.I64) {a} else {r}.skipValue()"
            );
        }
        PropType::Double => {
            let a = assign(&format!("{r}.readF64()"));
            writeln!(
                out,
                "{indent}if ({r}.peekTag() == BinTag.F64 || {r}.peekTag() == BinTag.I64) {a} else {r}.skipValue()"
            );
        }
        PropType::Color => {
            let a_s = assign(&format!("{r}.readStr()"));
            let a_i = assign(&format!(
                "String.format(java.util.Locale.ROOT, \"#%08X\", {r}.readI64() and 0xFFFF_FFFFL)"
            ));
            writeln!(
                out,
                "{indent}when ({r}.peekTag()) {{ BinTag.STR -> {a_s}; BinTag.I64 -> {a_i}; else -> {r}.skipValue() }}"
            );
        }
        PropType::Array { item } => {
            let am = format!("_am{depth}");
            let list = format!("_list{depth}");
            let idx = format!("_j{depth}");
            let elem_ty = kotlin_simple_type(item);
            let inner = format!("{indent}    ");
            let loop_inner = format!("{indent}        ");
            let a = assign(&list);
            writeln!(out, "{indent}if ({r}.peekTag() == BinTag.ARRAY) {{");
            writeln!(out, "{inner}val {am} = {r}.beginArray()");
            writeln!(out, "{inner}val {list} = ArrayList<{elem_ty}>({am}.count)");
            writeln!(out, "{inner}for ({idx} in 0 until {am}.count) {{");
            emit_compose_bin_read_value(out, &loop_inner, r, &list, true, item, false, depth + 1);
            writeln!(out, "{inner}}}");
            writeln!(out, "{inner}{r}.end({am})");
            writeln!(out, "{inner}{a}");
            writeln!(out, "{indent}}} else {{");
            writeln!(out, "{inner}{r}.skipValue()");
            writeln!(out, "{indent}}}");
        }
        PropType::Object { .. } => {
            let a = assign(&format!("HybridRuntime.DynamicValue.read({r})"));
            writeln!(out, "{indent}{a}");
        }
        _ => {
            let a = assign(&format!("{r}.readStr()"));
            writeln!(
                out,
                "{indent}if ({r}.peekTag() == BinTag.STR) {a} else {r}.skipValue()"
            );
        }
    }
}

/// Returns `(kotlin_param_type, react_prop_annotation)` for a React Native Android `@ReactProp` setter.
pub(super) fn rn_prop_signature(prop: &PropSpec) -> (String, String) {
    let key = escape_kotlin_string(&prop.name);
    match &prop.prop_type {
        PropType::Bool => {
            let def = match prop.default_value {
                Some(DefaultValue::Bool(true)) => "true",
                _ => "false",
            };
            (
                "Boolean".to_owned(),
                format!("@ReactProp(name = \"{key}\", defaultBoolean = {def})"),
            )
        }
        PropType::Int32 | PropType::IntEnum { .. } => {
            let def = match prop.default_value {
                Some(DefaultValue::Int(i)) => i32::try_from(i).unwrap_or(0),
                _ => 0,
            };
            (
                "Int".to_owned(),
                format!("@ReactProp(name = \"{key}\", defaultInt = {def})"),
            )
        }
        PropType::Float => {
            let def = match prop.default_value {
                Some(DefaultValue::Float(f)) => format!("{}f", format_float_literal(f)),
                Some(DefaultValue::Int(i)) => format!("{i}.0f"),
                _ => "0.0f".to_owned(),
            };
            (
                "Float".to_owned(),
                format!("@ReactProp(name = \"{key}\", defaultFloat = {def})"),
            )
        }
        PropType::Double => {
            let def = match prop.default_value {
                Some(DefaultValue::Float(f)) => format_float_literal(f),
                Some(DefaultValue::Int(i)) => format!("{i}.0"),
                _ => "0.0".to_owned(),
            };
            (
                "Double".to_owned(),
                format!("@ReactProp(name = \"{key}\", defaultDouble = {def})"),
            )
        }
        PropType::Color => (
            "Int?".to_owned(),
            format!("@ReactProp(name = \"{key}\", customType = \"Color\")"),
        ),
        PropType::Array { .. } => (
            "ReadableArray?".to_owned(),
            format!("@ReactProp(name = \"{key}\")"),
        ),
        PropType::Object { .. } | PropType::Point | PropType::EdgeInsets => (
            "ReadableMap?".to_owned(),
            format!("@ReactProp(name = \"{key}\")"),
        ),
        _ => (
            "String?".to_owned(),
            format!("@ReactProp(name = \"{key}\")"),
        ),
    }
}

/// Emits eager Kotlin statements encoding a React Native Android `@ReactProp` value (including
/// recursive `ReadableArray` / `ReadableMap` and unsigned 32-bit `Color`) directly into `BinWriter`
/// `w`. Runs synchronously inside the setter so no RN bridge reference is ever captured.
pub(super) fn emit_rn_write_prop_value(
    out: &mut CodeWriter,
    indent: &str,
    w: &str,
    val_expr: &str,
    ty: &PropType,
    depth: usize,
) {
    match ty {
        PropType::Bool => writeln!(out, "{indent}{w}.writeBool({val_expr})"),
        PropType::Int32 | PropType::IntEnum { .. } => {
            writeln!(out, "{indent}{w}.writeI64({val_expr}.toLong())")
        }
        PropType::Float => writeln!(out, "{indent}{w}.writeF64({val_expr}.toDouble())"),
        PropType::Double => writeln!(out, "{indent}{w}.writeF64({val_expr})"),
        PropType::Color => {
            writeln!(
                out,
                "{indent}if ({val_expr} != null) {w}.writeI64({val_expr}.toLong() and 0xFFFF_FFFFL) else {w}.writeNull()"
            );
        }
        PropType::Object { .. } | PropType::Point | PropType::EdgeInsets => {
            writeln!(out, "{indent}writeReadableMap({w}, {val_expr}, {depth})");
        }
        PropType::Array { item } => {
            let arr = format!("_arr{depth}");
            let mark = format!("_m{depth}");
            let idx = format!("_i{depth}");
            let inner = format!("{indent}    ");
            let loop_inner = format!("{indent}        ");
            let elem_inner = format!("{indent}            ");
            writeln!(out, "{indent}val {arr} = {val_expr}");
            writeln!(out, "{indent}if ({arr} == null) {{");
            writeln!(out, "{inner}{w}.writeNull()");
            writeln!(out, "{indent}}} else {{");
            writeln!(out, "{inner}val {mark} = {w}.beginArray({arr}.size())");
            writeln!(out, "{inner}for ({idx} in 0 until {arr}.size()) {{");
            writeln!(out, "{loop_inner}if ({arr}.isNull({idx})) {{");
            writeln!(out, "{elem_inner}{w}.writeNull()");
            writeln!(out, "{loop_inner}}} else {{");
            emit_rn_write_readable_array_elem(out, &elem_inner, w, &arr, &idx, item, depth + 1);
            writeln!(out, "{loop_inner}}}");
            writeln!(out, "{inner}}}");
            writeln!(out, "{inner}{w}.end({mark})");
            writeln!(out, "{indent}}}");
        }
        _ => {
            writeln!(
                out,
                "{indent}if ({val_expr} != null) {w}.writeStr({val_expr}) else {w}.writeNull()"
            );
        }
    }
}

/// Emits Kotlin statements encoding one non-null element at `arr_expr[idx_expr]` from a React Native
/// `ReadableArray` into `BinWriter` `w`. Shared by nested `Array<...>` props and `receiveCommand`.
pub(super) fn emit_rn_write_readable_array_elem(
    out: &mut CodeWriter,
    indent: &str,
    w: &str,
    arr_expr: &str,
    idx_expr: &str,
    ty: &PropType,
    depth: usize,
) {
    match ty {
        PropType::Bool => writeln!(out, "{indent}{w}.writeBool({arr_expr}.getBoolean({idx_expr}))"),
        PropType::Int32 | PropType::IntEnum { .. } => {
            writeln!(out, "{indent}{w}.writeI64({arr_expr}.getInt({idx_expr}).toLong())")
        }
        PropType::Float | PropType::Double => {
            writeln!(out, "{indent}{w}.writeF64({arr_expr}.getDouble({idx_expr}))")
        }
        PropType::Color => {
            writeln!(
                out,
                "{indent}{w}.writeI64({arr_expr}.getInt({idx_expr}).toLong() and 0xFFFF_FFFFL)"
            );
        }
        PropType::Object { .. } | PropType::Point | PropType::EdgeInsets => {
            writeln!(
                out,
                "{indent}writeReadableMap({w}, {arr_expr}.getMap({idx_expr}), {depth})"
            );
        }
        PropType::Array { .. } => {
            let sub = format!("{arr_expr}.getArray({idx_expr})");
            emit_rn_write_prop_value(out, indent, w, &sub, ty, depth);
        }
        _ => {
            writeln!(
                out,
                "{indent}{arr_expr}.getString({idx_expr})?.let {{ {w}.writeStr(it) }} ?: {w}.writeNull()"
            );
        }
    }
}

/// Emits one `when (_r.readMapKey())` arm decoding an event payload field from `BinReader` `r`
/// into React Native `WritableMap` `event_map`.
pub(super) fn emit_rn_read_event_field(
    out: &mut CodeWriter,
    indent: &str,
    r: &str,
    event_map: &str,
    fkey: &str,
    f: &ObjectField,
) {
    match &f.field_type {
        PropType::Bool => {
            writeln!(
                out,
                "{indent}\"{fkey}\" -> when ({r}.peekTag()) {{ BinTag.BOOL -> {event_map}.putBoolean(\"{fkey}\", {r}.readBool()); BinTag.NULL -> {{ {r}.readNull(); {event_map}.putNull(\"{fkey}\") }}; else -> {r}.skipValue() }}"
            );
        }
        PropType::Int32 | PropType::IntEnum { .. } => {
            writeln!(
                out,
                "{indent}\"{fkey}\" -> when ({r}.peekTag()) {{ BinTag.I64 -> {event_map}.putInt(\"{fkey}\", {r}.readI64().toInt()); BinTag.F64 -> {event_map}.putInt(\"{fkey}\", {r}.readF64().toInt()); BinTag.NULL -> {{ {r}.readNull(); {event_map}.putNull(\"{fkey}\") }}; else -> {r}.skipValue() }}"
            );
        }
        PropType::Float | PropType::Double => {
            writeln!(
                out,
                "{indent}\"{fkey}\" -> when ({r}.peekTag()) {{ BinTag.F64, BinTag.I64 -> {event_map}.putDouble(\"{fkey}\", {r}.readF64()); BinTag.NULL -> {{ {r}.readNull(); {event_map}.putNull(\"{fkey}\") }}; else -> {r}.skipValue() }}"
            );
        }
        PropType::Color => {
            writeln!(
                out,
                "{indent}\"{fkey}\" -> when ({r}.peekTag()) {{ BinTag.I64 -> {event_map}.putInt(\"{fkey}\", ({r}.readI64() and 0xFFFF_FFFFL).toInt()); BinTag.STR -> {event_map}.putString(\"{fkey}\", {r}.readStr()); BinTag.NULL -> {{ {r}.readNull(); {event_map}.putNull(\"{fkey}\") }}; else -> {r}.skipValue() }}"
            );
        }
        PropType::Array { .. } => {
            writeln!(
                out,
                "{indent}\"{fkey}\" -> when ({r}.peekTag()) {{ BinTag.ARRAY -> {event_map}.putArray(\"{fkey}\", readBinToWritableArray({r}, 0)); BinTag.NULL -> {{ {r}.readNull(); {event_map}.putNull(\"{fkey}\") }}; else -> {r}.skipValue() }}"
            );
        }
        PropType::Object { .. } | PropType::Point | PropType::EdgeInsets | PropType::Mixed => {
            writeln!(
                out,
                "{indent}\"{fkey}\" -> putBinValue({event_map}, \"{fkey}\", {r}, 0)"
            );
        }
        _ => {
            writeln!(
                out,
                "{indent}\"{fkey}\" -> when ({r}.peekTag()) {{ BinTag.STR -> {event_map}.putString(\"{fkey}\", {r}.readStr()); BinTag.NULL -> {{ {r}.readNull(); {event_map}.putNull(\"{fkey}\") }}; else -> {r}.skipValue() }}"
            );
        }
    }
}

fn emit_rn_readable_type_switch(out: &mut CodeWriter, src: &str, key: &str) {
    writeln!(out, "            if ({src}.isNull({key})) {{");
    writeln!(out, "                _w.writeNull()");
    writeln!(out, "            }} else when ({src}.getType({key})) {{");
    writeln!(out, "                ReadableType.Null -> _w.writeNull()");
    writeln!(out, "                ReadableType.Boolean -> _w.writeBool({src}.getBoolean({key}))");
    writeln!(out, "                ReadableType.Number -> _w.writeF64({src}.getDouble({key}))");
    writeln!(out, "                ReadableType.String -> {src}.getString({key})?.let {{ _w.writeStr(it) }} ?: _w.writeNull()");
    writeln!(out, "                ReadableType.Array -> writeReadableArrayDynamic(_w, {src}.getArray({key}), _depth + 1)");
    writeln!(out, "                ReadableType.Map -> writeReadableMap(_w, {src}.getMap({key}), _depth + 1)");
    writeln!(out, "            }}");
}

fn emit_rn_bin_tag_writer(out: &mut CodeWriter, target: &str, verb: &str, key_arg: &str) {
    let ka = if key_arg.is_empty() { String::new() } else { format!("{key_arg}, ") };
    writeln!(out, "        when (_r.peekTag()) {{");
    writeln!(out, "            BinTag.NULL -> {{ _r.readNull(); {target}.{verb}Null({key_arg}) }}");
    writeln!(out, "            BinTag.BOOL -> {target}.{verb}Boolean({ka}_r.readBool())");
    writeln!(out, "            BinTag.I64 -> {{");
    writeln!(out, "                val _v = _r.readI64()");
    writeln!(out, "                if (_v in Int.MIN_VALUE..Int.MAX_VALUE) {target}.{verb}Int({ka}_v.toInt()) else {target}.{verb}Double({ka}_v.toDouble())");
    writeln!(out, "            }}");
    writeln!(out, "            BinTag.F64 -> {target}.{verb}Double({ka}_r.readF64())");
    writeln!(out, "            BinTag.STR -> {target}.{verb}String({ka}_r.readStr())");
    writeln!(out, "            BinTag.BYTES -> {{");
    writeln!(out, "                val _bytes = _r.readBytes()");
    writeln!(out, "                val _ba = Arguments.createArray()");
    writeln!(out, "                for (_b in _bytes) _ba.pushInt(_b.toInt() and 0xFF)");
    writeln!(out, "                {target}.{verb}Array({ka}_ba)");
    writeln!(out, "            }}");
    writeln!(out, "            BinTag.ARRAY -> {target}.{verb}Array({ka}readBinToWritableArray(_r, _depth))");
    writeln!(out, "            BinTag.MAP -> {target}.{verb}Map({ka}readBinToWritableMap(_r, _depth))");
    writeln!(out, "        }}");
}

/// Emits private recursive RN bridge codec helpers inside `<Name>ViewManager` only when used.
pub(super) fn emit_rn_dynamic_helpers(
    out: &mut CodeWriter,
    emit_readable_writers: bool,
    emit_writable_readers: bool,
) {
    if emit_readable_writers {
        writeln!(out);
        writeln!(out, "    private fun writeReadableMap(_w: BinWriter, _map: ReadableMap?, _depth: Int) {{");
        writeln!(out, "        if (_map == null) {{ _w.writeNull(); return }}");
        writeln!(out, "        if (_depth >= BinTag.MAX_DEPTH) throw BinCodecException(BinCodecException.Kind.MAX_DEPTH, \"depth $_depth\")");
        writeln!(out, "        val _keys = ArrayList<String>()");
        writeln!(out, "        val _it = _map.keySetIterator()");
        writeln!(out, "        while (_it.hasNextKey()) _keys.add(_it.nextKey())");
        writeln!(out, "        val _m = _w.beginMap(_keys.size)");
        writeln!(out, "        for (_k in _keys) {{");
        writeln!(out, "            _w.writeMapKey(_k)");
        emit_rn_readable_type_switch(out, "_map", "_k");
        writeln!(out, "        }}");
        writeln!(out, "        _w.end(_m)");
        writeln!(out, "    }}");
        writeln!(out);
        writeln!(out, "    private fun writeReadableArrayDynamic(_w: BinWriter, _arr: ReadableArray?, _depth: Int) {{");
        writeln!(out, "        if (_arr == null) {{ _w.writeNull(); return }}");
        writeln!(out, "        if (_depth >= BinTag.MAX_DEPTH) throw BinCodecException(BinCodecException.Kind.MAX_DEPTH, \"depth $_depth\")");
        writeln!(out, "        val _m = _w.beginArray(_arr.size())");
        writeln!(out, "        for (_i in 0 until _arr.size()) {{");
        emit_rn_readable_type_switch(out, "_arr", "_i");
        writeln!(out, "        }}");
        writeln!(out, "        _w.end(_m)");
        writeln!(out, "    }}");
    }

    if emit_writable_readers {
        writeln!(out);
        writeln!(out, "    private fun readBinToWritableMap(_r: BinReader, _depth: Int): WritableMap {{");
        writeln!(out, "        if (_depth >= BinTag.MAX_DEPTH) throw BinCodecException(BinCodecException.Kind.MAX_DEPTH, \"depth $_depth\")");
        writeln!(out, "        val _out = Arguments.createMap()");
        writeln!(out, "        val _m = _r.beginMap()");
        writeln!(out, "        for (_i in 0 until _m.count) putBinValue(_out, _r.readMapKey(), _r, _depth + 1)");
        writeln!(out, "        _r.end(_m)");
        writeln!(out, "        return _out");
        writeln!(out, "    }}");
        writeln!(out);
        writeln!(out, "    private fun readBinToWritableArray(_r: BinReader, _depth: Int): WritableArray {{");
        writeln!(out, "        if (_depth >= BinTag.MAX_DEPTH) throw BinCodecException(BinCodecException.Kind.MAX_DEPTH, \"depth $_depth\")");
        writeln!(out, "        val _out = Arguments.createArray()");
        writeln!(out, "        val _m = _r.beginArray()");
        writeln!(out, "        for (_i in 0 until _m.count) pushBinValue(_out, _r, _depth + 1)");
        writeln!(out, "        _r.end(_m)");
        writeln!(out, "        return _out");
        writeln!(out, "    }}");
        writeln!(out);
        writeln!(out, "    private fun putBinValue(_map: WritableMap, _key: String, _r: BinReader, _depth: Int) {{");
        emit_rn_bin_tag_writer(out, "_map", "put", "_key");
        writeln!(out, "    }}");
        writeln!(out);
        writeln!(out, "    private fun pushBinValue(_arr: WritableArray, _r: BinReader, _depth: Int) {{");
        emit_rn_bin_tag_writer(out, "_arr", "push", "");
        writeln!(out, "    }}");
    }
}
