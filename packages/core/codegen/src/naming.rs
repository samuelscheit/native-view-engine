//! Identifier and case-conversion utilities for Rust, TypeScript, C++20, ObjC++, and Kotlin.

/// Converts `PascalCase`, `camelCase`, `kebab-case`, or acronym-prefixed names
/// (e.g. `RCTModalHostView`, `testID`, `portrait-upside-down`) into `snake_case`.
pub fn to_snake_case(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 8);
    let mut prev_was_underscore = true;
    let mut prev: Option<char> = None;
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if !ch.is_ascii_alphanumeric() {
            if !prev_was_underscore && !out.is_empty() {
                out.push('_');
                prev_was_underscore = true;
            }
            prev = Some(ch);
            continue;
        }
        if ch.is_ascii_uppercase() {
            let prev_is_lower_or_digit =
                prev.is_some_and(|p| p.is_ascii_lowercase() || p.is_ascii_digit());
            let next_is_lower = chars.peek().is_some_and(char::is_ascii_lowercase);
            let prev_is_upper = prev.is_some_and(|p| p.is_ascii_uppercase());
            if !out.is_empty()
                && !prev_was_underscore
                && (prev_is_lower_or_digit || (prev_is_upper && next_is_lower))
            {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
            prev_was_underscore = false;
        } else {
            out.push(ch.to_ascii_lowercase());
            prev_was_underscore = false;
        }
        prev = Some(ch);
    }

    while out.ends_with('_') {
        out.pop();
    }
    if out.is_empty() {
        "unnamed".to_owned()
    } else {
        out
    }
}

/// Converts any identifier or hyphenated string literal (e.g. `'portrait-upside-down'`,
/// `on_count_change`, `small`) into `PascalCase`.
pub fn to_pascal_case(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut capitalize_next = true;

    for ch in input.chars() {
        if !ch.is_ascii_alphanumeric() {
            capitalize_next = true;
            continue;
        }
        if capitalize_next {
            out.push(ch.to_ascii_uppercase());
            capitalize_next = false;
        } else {
            out.push(ch);
        }
    }

    if out.is_empty() {
        "Unnamed".to_owned()
    } else if out.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        format!("V{out}")
    } else {
        out
    }
}

/// Converts an identifier into `camelCase`.
pub fn to_camel_case(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut capitalize_next = false;

    for ch in input.chars() {
        if !ch.is_ascii_alphanumeric() {
            if !out.is_empty() {
                capitalize_next = true;
            }
            continue;
        }
        if out.is_empty() {
            out.push(ch.to_ascii_lowercase());
            capitalize_next = false;
        } else if capitalize_next {
            out.push(ch.to_ascii_uppercase());
            capitalize_next = false;
        } else {
            out.push(ch);
        }
    }

    if out.is_empty() {
        "unnamed".to_owned()
    } else if out.as_bytes()[0].is_ascii_digit() {
        format!("v{out}")
    } else {
        out
    }
}

/// Converts a `SCREAMING_SNAKE_CASE` constant name into `camelCase` without allocating an
/// intermediate lowercased `String`.
pub fn screaming_snake_to_camel_case(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut capitalize_next = false;
    for ch in input.chars() {
        if !ch.is_ascii_alphanumeric() {
            if !out.is_empty() {
                capitalize_next = true;
            }
            continue;
        }
        if capitalize_next {
            out.push(ch.to_ascii_uppercase());
            capitalize_next = false;
        } else {
            out.push(ch.to_ascii_lowercase());
        }
    }
    if out.is_empty() {
        "unnamed".to_owned()
    } else if out.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        format!("v{out}")
    } else {
        out
    }
}

/// Strips a leading `on` prefix from an event name if followed by an uppercase letter
/// (e.g. `onCountChange` -> `countChange`, `onChange` -> `change`).
pub fn strip_on_prefix(event_name: &str) -> String {
    if let Some(rest) = event_name.strip_prefix("on") {
        let mut chars = rest.chars();
        if let Some(first) = chars.next()
            && first.is_ascii_uppercase()
        {
            let mut out = String::with_capacity(rest.len());
            out.push(first.to_ascii_lowercase());
            out.push_str(chars.as_str());
            return out;
        }
    }
    to_camel_case(event_name)
}

/// Escapes reserved Rust keywords or leading-digit identifiers.
pub fn rust_ident(raw: &str) -> String {
    let mut ident = to_snake_case(raw);
    if ident.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        ident.insert(0, '_');
    }
    match ident.as_str() {
        "as" | "async" | "await" | "break" | "const" | "continue" | "crate" | "dyn" | "else"
        | "enum" | "extern" | "false" | "fn" | "for" | "if" | "impl" | "in" | "let" | "loop"
        | "match" | "mod" | "move" | "mut" | "pub" | "ref" | "return" | "static" | "struct"
        | "trait" | "true" | "type" | "unsafe" | "use" | "where" | "while" | "abstract"
        | "become" | "box" | "do" | "final" | "macro" | "override" | "priv" | "try" | "typeof"
        | "unsized" | "virtual" | "yield" => format!("r#{ident}"),
        "self" | "Self" | "super" => format!("{ident}_"),
        _ => ident,
    }
}

/// Escapes reserved C++20 keywords.
pub fn cpp_ident(raw: &str) -> String {
    let mut ident = to_camel_case(raw);
    if ident.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        ident.insert(0, '_');
    }
    match ident.as_str() {
        "alignas" | "alignof" | "and" | "asm" | "auto" | "bool" | "break" | "case" | "catch"
        | "char" | "class" | "concept" | "const" | "consteval" | "constexpr" | "constinit"
        | "continue" | "default" | "delete" | "do" | "double" | "else" | "enum" | "explicit"
        | "export" | "extern" | "false" | "float" | "for" | "friend" | "goto" | "if" | "inline"
        | "int" | "long" | "mutable" | "namespace" | "new" | "noexcept" | "not" | "nullptr"
        | "operator" | "or" | "private" | "protected" | "public" | "register" | "requires"
        | "return" | "short" | "signed" | "sizeof" | "static" | "struct" | "switch"
        | "template" | "this" | "throw" | "true" | "try" | "typedef" | "typeid" | "typename"
        | "union" | "unsigned" | "using" | "virtual" | "void" | "volatile" | "while" => {
            format!("{ident}_")
        }
        _ => ident,
    }
}

/// Escapes reserved Kotlin keywords.
pub fn kotlin_ident(raw: &str) -> String {
    let mut ident = to_camel_case(raw);
    if ident.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        ident.insert(0, '_');
    }
    match ident.as_str() {
        "as" | "break" | "class" | "continue" | "do" | "else" | "false" | "for" | "fun" | "if"
        | "in" | "interface" | "is" | "null" | "object" | "package" | "return" | "super"
        | "this" | "throw" | "true" | "try" | "typealias" | "typeof" | "val" | "var" | "when"
        | "while" => format!("`{ident}`"),
        _ => ident,
    }
}

/// Escapes reserved Dart keywords.
pub fn dart_ident(raw: &str) -> String {
    let mut ident = to_camel_case(raw);
    if ident.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        ident.insert(0, '_');
    }
    match ident.as_str() {
        "abstract" | "as" | "assert" | "async" | "await" | "base" | "break" | "case" | "catch"
        | "class" | "const" | "continue" | "covariant" | "default" | "deferred" | "do"
        | "dynamic" | "else" | "enum" | "export" | "extends" | "extension" | "external"
        | "factory" | "false" | "final" | "finally" | "for" | "Function" | "get" | "hide" | "if"
        | "implements" | "import" | "in" | "interface" | "is" | "late" | "library" | "mixin"
        | "new" | "null" | "on" | "operator" | "part" | "required" | "rethrow" | "return"
        | "sealed" | "set" | "show" | "static" | "super" | "switch" | "sync" | "this" | "throw"
        | "true" | "try" | "typedef" | "var" | "void" | "when" | "while" | "with" | "yield" => {
            format!("{ident}Value")
        }
        _ => ident,
    }
}

/// Escapes reserved Swift keywords.
pub fn swift_ident(raw: &str) -> String {
    let mut ident = to_camel_case(raw);
    if ident.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        ident.insert(0, '_');
    }
    match ident.as_str() {
        "associatedtype" | "class" | "deinit" | "enum" | "extension" | "fileprivate" | "func"
        | "import" | "init" | "inout" | "internal" | "let" | "open" | "operator" | "private"
        | "precedencegroup" | "protocol" | "public" | "rethrows" | "static" | "struct"
        | "subscript" | "typealias" | "var" | "break" | "case" | "catch" | "continue"
        | "default" | "defer" | "do" | "else" | "fallthrough" | "for" | "guard" | "if" | "in"
        | "repeat" | "return" | "throw" | "switch" | "where" | "while" | "as" | "false" | "is"
        | "nil" | "self" | "Self" | "super" | "throws" | "true" | "try" => format!("`{ident}`"),
        _ => ident,
    }
}

/// Converts any identifier into `kebab-case` (e.g. `NveCounterCard` -> `nve-counter-card`).
pub fn to_kebab_case(input: &str) -> String {
    to_snake_case(input).replace('_', "-")
}

/// Returns a valid Custom Element tag name prefixed with `nve-`
/// (e.g. `NveCounterCard` -> `nve-counter-card`, `CounterCard` -> `nve-counter-card`).
pub fn web_component_tag(component_name: &str) -> String {
    let kebab = to_kebab_case(component_name);
    if kebab.starts_with("nve-") {
        kebab
    } else {
        format!("nve-{kebab}")
    }
}

/// Escapes reserved C11/C++20 keywords for snake_case C struct/vtable field names.
pub fn c_field_ident(raw: &str) -> String {
    let mut s = to_snake_case(raw);
    if s.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        s.insert(0, '_');
    }
    match s.as_str() {
        "auto" | "break" | "case" | "char" | "const" | "continue" | "default" | "do" | "double"
        | "else" | "enum" | "extern" | "float" | "for" | "goto" | "if" | "inline" | "int"
        | "long" | "register" | "restrict" | "return" | "short" | "signed" | "sizeof"
        | "static" | "struct" | "switch" | "typedef" | "union" | "unsigned" | "void"
        | "volatile" | "while" | "class" | "delete" | "new" | "namespace" | "template" => {
            format!("{s}_")
        }
        _ => s,
    }
}

/// Formats an integer enum discriminant as a `PascalCase` variant identifier (`Neg1` / `V0`).
pub fn int_enum_variant_pascal(v: i32) -> String {
    if v < 0 {
        format!("Neg{}", v.unsigned_abs())
    } else {
        format!("V{v}")
    }
}

/// Formats an integer enum discriminant as a `camelCase` variant identifier (`neg1` / `v0`).
pub fn int_enum_variant_camel(v: i32) -> String {
    if v < 0 {
        format!("neg{}", v.unsigned_abs())
    } else {
        format!("v{v}")
    }
}

/// Escapes a string literal for C11, C++20, ObjC++, Rust, and Swift double-quoted literals.
///
/// Escaping `\` -> `\\` also prevents Swift `\(...)` string interpolation.
pub fn escape_c_like_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\0' => out.push_str("\\0"),
            c => out.push(c),
        }
    }
    out
}

/// Escapes a string literal for Kotlin double-quoted literals.
///
/// Escapes `$` -> `\$` so `"$HOME"` and `"${expr}"` never trigger Kotlin string interpolation,
/// and escapes `\0` as `\u0000` (Kotlin does not accept `\0`).
pub fn escape_kotlin_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '$' => out.push_str("\\$"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\0' => out.push_str("\\u0000"),
            c => out.push(c),
        }
    }
    out
}

/// Escapes a string literal for Dart single-quoted literals.
///
/// Escapes `$` -> `\$` so `'$HOME'` and `'${expr}'` never trigger Dart string interpolation,
/// and escapes `\0` as `\u0000`.
pub fn escape_dart_single_quoted_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '$' => out.push_str("\\$"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\0' => out.push_str("\\u0000"),
            c => out.push(c),
        }
    }
    out
}

/// Sanitizes untrusted text for inclusion in a single-line (`//` or `///`) comment.
///
/// Strips CR/LF and Unicode line separators so comment breakout onto a new line is impossible.
pub fn sanitize_single_line_comment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_sep = false;
    for ch in s.chars() {
        if matches!(ch, '\r' | '\n' | '\u{2028}' | '\u{2029}') {
            if !prev_sep && !out.is_empty() {
                out.push(' ');
            }
            prev_sep = true;
        } else {
            out.push(ch);
            prev_sep = false;
        }
    }
    out.trim_end().to_owned()
}

/// Sanitizes untrusted text for inclusion in a block comment (`/* ... */` or `/** ... */`).
///
/// Breaks any `*/` sequence and strips `\r` so block-comment breakout is impossible.
pub fn sanitize_block_comment(s: &str) -> String {
    s.replace('\r', "").replace("*/", "* /")
}

/// Formats an `f64` so integral values always include `.0` (valid in C++, Kotlin, Swift, Dart, TS).
pub fn format_float_literal(f: f64) -> String {
    if !f.is_finite() {
        return "0.0".to_owned();
    }
    let s = f.to_string();
    if s.contains('.') || s.contains('e') || s.contains('E') {
        s
    } else {
        format!("{s}.0")
    }
}

/// Escapes a string literal for TypeScript/JavaScript single-quoted literals.
pub fn escape_ts_single_quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\0' => out.push_str("\\u0000"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{to_camel_case, to_pascal_case, to_snake_case};

    #[test]
    fn snake_case_handles_acronyms_separators_and_unicode() {
        for (input, expected) in [
            ("", "unnamed"),
            ("___", "unnamed"),
            ("RCTModalHostView", "rct_modal_host_view"),
            ("testID", "test_id"),
            ("portrait-upside-down", "portrait_upside_down"),
            ("foo1Bar", "foo1_bar"),
            ("a__b___", "a_b"),
            ("aéB", "a_b"),
            ("ABé", "ab"),
        ] {
            assert_eq!(to_snake_case(input), expected, "input={input:?}");
        }
    }

    #[test]
    fn camel_case_matches_lowercased_pascal_case() {
        for input in [
            "",
            "___",
            "a",
            "A",
            "ABc",
            "foo_bar",
            "foo__bar_",
            "_foo",
            "1foo",
            "1Foo",
            "1foo_bar",
            "_1_a",
            "aéB",
            "éa",
            "testID",
        ] {
            let pascal = to_pascal_case(input);
            let mut chars = pascal.chars();
            let expected = match chars.next() {
                None => "unnamed".to_owned(),
                Some(first) => format!("{}{}", first.to_ascii_lowercase(), chars.as_str()),
            };
            assert_eq!(to_camel_case(input), expected, "input={input:?}");
        }
    }
}
