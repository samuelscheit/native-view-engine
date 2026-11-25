//! Identifier and case-conversion utilities for Rust, TypeScript, C++20, ObjC++, and Kotlin.

/// Converts `PascalCase`, `camelCase`, `kebab-case`, or acronym-prefixed names
/// (e.g. `RCTModalHostView`, `testID`, `portrait-upside-down`) into `snake_case`.
pub fn to_snake_case(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len() + 8);
    let mut prev_was_underscore = true;

    for (i, &ch) in chars.iter().enumerate() {
        if !ch.is_ascii_alphanumeric() {
            if !prev_was_underscore && !out.is_empty() {
                out.push('_');
                prev_was_underscore = true;
            }
            continue;
        }
        if ch.is_ascii_uppercase() {
            let prev_is_lower_or_digit = i > 0
                && (chars[i - 1].is_ascii_lowercase() || chars[i - 1].is_ascii_digit());
            let next_is_lower = i + 1 < chars.len() && chars[i + 1].is_ascii_lowercase();
            let prev_is_upper = i > 0 && chars[i - 1].is_ascii_uppercase();
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
    let mut prev_was_lower = false;

    for ch in input.chars() {
        if !ch.is_ascii_alphanumeric() {
            capitalize_next = true;
            prev_was_lower = false;
            continue;
        }
        if capitalize_next {
            out.push(ch.to_ascii_uppercase());
            capitalize_next = false;
            prev_was_lower = false;
        } else if ch.is_ascii_uppercase() && prev_was_lower {
            out.push(ch);
            prev_was_lower = false;
        } else {
            out.push(ch);
            prev_was_lower = ch.is_ascii_lowercase() || ch.is_ascii_digit();
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
    let pascal = to_pascal_case(input);
    let mut chars = pascal.chars();
    match chars.next() {
        None => "unnamed".to_owned(),
        Some(first) => {
            let mut out = String::with_capacity(pascal.len());
            out.push(first.to_ascii_lowercase());
            out.push_str(chars.as_str());
            out
        }
    }
}

/// Strips a leading `on` prefix from an event name if followed by an uppercase letter
/// (e.g. `onCountChange` -> `countChange`, `onChange` -> `change`).
pub fn strip_on_prefix(event_name: &str) -> String {
    if let Some(rest) = event_name.strip_prefix("on") {
        let mut chars = rest.chars();
        if let Some(first) = chars.next() {
            if first.is_ascii_uppercase() {
                let mut out = String::with_capacity(rest.len());
                out.push(first.to_ascii_lowercase());
                out.push_str(chars.as_str());
                return out;
            }
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

/// Escapes a string literal for Rust/C++/TS/Kotlin double-quoted literals.
pub fn escape_string_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out
}

/// Formats an `f64` so integral values always include `.0` (valid in C++, Kotlin, Swift, Dart, TS).
pub fn format_float_literal(f: f64) -> String {
    let s = f.to_string();
    if s.contains('.') || s.contains('e') || s.contains('E') {
        s
    } else {
        format!("{s}.0")
    }
}

/// Escapes a string literal for TypeScript single-quoted literals.
pub fn escape_ts_single_quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out
}
