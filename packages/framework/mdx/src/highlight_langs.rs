//! Language definitions and keyword/type tables for the pure-Rust syntax highlighter.

/// Supported syntax highlighting languages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    Rust,
    Cpp,
    Swift,
    Kotlin,
    Java,
    Dart,
    TypeScript,
    Shell,
    Toml,
    Json,
    Cmake,
    Diff,
    Plain,
}

impl Language {
    /// Resolve a fenced code block language tag (case-insensitive) to a [`Language`].
    pub fn from_tag(tag: Option<&str>) -> Self {
        let Some(raw) = tag else {
            return Self::Plain;
        };
        let s = raw.trim();
        const TABLE: &[(&str, Language)] = &[
            ("rs", Language::Rust),
            ("rust", Language::Rust),
            ("rsx", Language::Rust),
            ("c", Language::Cpp),
            ("h", Language::Cpp),
            ("cpp", Language::Cpp),
            ("c++", Language::Cpp),
            ("cxx", Language::Cpp),
            ("cc", Language::Cpp),
            ("hpp", Language::Cpp),
            ("hxx", Language::Cpp),
            ("objc", Language::Cpp),
            ("objcpp", Language::Cpp),
            ("objc++", Language::Cpp),
            ("mm", Language::Cpp),
            ("m", Language::Cpp),
            ("swift", Language::Swift),
            ("kt", Language::Kotlin),
            ("kts", Language::Kotlin),
            ("kotlin", Language::Kotlin),
            ("java", Language::Java),
            ("dart", Language::Dart),
            ("ts", Language::TypeScript),
            ("tsx", Language::TypeScript),
            ("js", Language::TypeScript),
            ("jsx", Language::TypeScript),
            ("mjs", Language::TypeScript),
            ("cjs", Language::TypeScript),
            ("typescript", Language::TypeScript),
            ("javascript", Language::TypeScript),
            ("sh", Language::Shell),
            ("bash", Language::Shell),
            ("zsh", Language::Shell),
            ("shell", Language::Shell),
            ("console", Language::Shell),
            ("toml", Language::Toml),
            ("json", Language::Json),
            ("jsonc", Language::Json),
            ("cmake", Language::Cmake),
            ("cmakelists", Language::Cmake),
            ("diff", Language::Diff),
            ("patch", Language::Diff),
        ];
        TABLE
            .iter()
            .find_map(|&(name, lang)| s.eq_ignore_ascii_case(name).then_some(lang))
            .unwrap_or(Self::Plain)
    }

    pub const fn supports_slash_comments(self) -> bool {
        matches!(
            self,
            Self::Rust
                | Self::Cpp
                | Self::Swift
                | Self::Kotlin
                | Self::Java
                | Self::Dart
                | Self::TypeScript
                | Self::Json
        )
    }

    pub const fn supports_nested_block_comments(self) -> bool {
        matches!(self, Self::Rust | Self::Swift)
    }

    pub const fn supports_hash_comments(self) -> bool {
        matches!(self, Self::Shell | Self::Toml | Self::Cmake)
    }

    pub const fn supports_single_quote_strings(self) -> bool {
        matches!(
            self,
            Self::TypeScript | Self::Dart | Self::Shell | Self::Toml
        )
    }

    pub const fn supports_backtick_strings(self) -> bool {
        matches!(self, Self::TypeScript | Self::Shell)
    }

    pub const fn supports_triple_quotes(self) -> bool {
        matches!(self, Self::Swift | Self::Kotlin | Self::Dart | Self::Toml)
    }

    pub fn is_keyword(self, word: &str) -> bool {
        let list: &[&str] = match self {
            Self::Rust => RUST_KEYWORDS,
            Self::Cpp => CPP_KEYWORDS,
            Self::Swift => SWIFT_KEYWORDS,
            Self::Kotlin => KOTLIN_KEYWORDS,
            Self::Java => JAVA_KEYWORDS,
            Self::Dart => DART_KEYWORDS,
            Self::TypeScript => TS_KEYWORDS,
            Self::Shell => SHELL_KEYWORDS,
            Self::Toml => TOML_KEYWORDS,
            Self::Json => JSON_KEYWORDS,
            Self::Cmake => CMAKE_KEYWORDS,
            Self::Diff | Self::Plain => return false,
        };
        list.contains(&word)
    }

    pub fn is_builtin_type(self, word: &str) -> bool {
        let list: &[&str] = match self {
            Self::Rust => RUST_TYPES,
            Self::Cpp => CPP_TYPES,
            Self::Swift => SWIFT_TYPES,
            Self::Kotlin => KOTLIN_TYPES,
            Self::Java => JAVA_TYPES,
            Self::Dart => DART_TYPES,
            Self::TypeScript => TS_TYPES,
            _ => return false,
        };
        list.contains(&word)
    }
}

const RUST_KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum",
    "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move",
    "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true",
    "type", "unsafe", "use", "where", "while",
];

const RUST_TYPES: &[&str] = &[
    "bool", "char", "str", "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64",
    "i128", "isize", "f32", "f64", "String", "Vec", "Option", "Result", "Box", "Rc", "Arc",
    "BTreeMap", "HashMap", "Element", "Signal",
];

const CPP_KEYWORDS: &[&str] = &[
    "alignas", "alignof", "auto", "bool", "break", "case", "catch", "char", "class", "co_await",
    "co_return", "co_yield", "concept", "const", "consteval", "constexpr", "constinit", "continue",
    "default", "delete", "do", "double", "else", "enum", "explicit", "export", "extern", "false",
    "float", "for", "friend", "goto", "if", "inline", "int", "long", "mutable", "namespace", "new",
    "noexcept", "nullptr", "operator", "override", "private", "protected", "public", "requires",
    "return", "short", "signed", "sizeof", "static", "static_assert", "static_cast",
    "reinterpret_cast", "struct", "switch", "template", "this", "throw", "true", "try", "typedef",
    "typename", "union", "unsigned", "using", "virtual", "void", "volatile", "while", "YES", "NO",
    "nil", "self", "super", "id", "instancetype",
];

const CPP_TYPES: &[&str] = &[
    "size_t", "ssize_t", "ptrdiff_t", "int8_t", "int16_t", "int32_t", "int64_t", "uint8_t",
    "uint16_t", "uint32_t", "uint64_t", "uintptr_t", "intptr_t", "std", "string", "string_view",
    "vector", "span", "unique_ptr", "shared_ptr", "NSString", "NSView", "UIView", "CALayer",
];

const SWIFT_KEYWORDS: &[&str] = &[
    "actor", "any", "as", "associatedtype", "async", "await", "break", "case", "catch", "class",
    "continue", "default", "defer", "deinit", "do", "else", "enum", "extension", "fallthrough",
    "false", "fileprivate", "for", "func", "guard", "if", "import", "in", "init", "inout",
    "internal", "is", "let", "mutating", "nil", "nonisolated", "open", "operator", "override",
    "private", "protocol", "public", "repeat", "rethrows", "return", "self", "Self", "some",
    "static", "struct", "subscript", "super", "switch", "throw", "throws", "true", "try",
    "typealias", "var", "weak", "where", "while",
];

const SWIFT_TYPES: &[&str] = &[
    "Int", "Int8", "Int16", "Int32", "Int64", "UInt", "UInt8", "UInt16", "UInt32", "UInt64",
    "Float", "Double", "Bool", "String", "Character", "Array", "Dictionary", "Set", "Optional",
    "Data", "Void", "View", "CGFloat", "CGRect", "UnsafePointer", "UnsafeRawPointer",
];

const KOTLIN_KEYWORDS: &[&str] = &[
    "abstract", "actual", "annotation", "as", "break", "by", "catch", "class", "companion",
    "const", "constructor", "continue", "crossinline", "data", "do", "else", "enum", "expect",
    "external", "false", "final", "finally", "for", "fun", "if", "import", "in", "infix", "init",
    "inline", "inner", "interface", "internal", "is", "lateinit", "noinline", "null", "object",
    "open", "operator", "out", "override", "package", "private", "protected", "public", "reified",
    "return", "sealed", "super", "suspend", "tailrec", "this", "throw", "true", "try", "typealias",
    "val", "var", "vararg", "when", "where", "while",
];

const KOTLIN_TYPES: &[&str] = &[
    "Unit", "Nothing", "Any", "Boolean", "Byte", "Short", "Int", "Long", "Float", "Double", "Char",
    "String", "List", "MutableList", "Map", "Set", "ByteArray", "IntArray", "Modifier",
];

const JAVA_KEYWORDS: &[&str] = &[
    "abstract", "assert", "boolean", "break", "byte", "case", "catch", "char", "class", "continue",
    "default", "do", "double", "else", "enum", "extends", "false", "final", "finally", "float",
    "for", "if", "implements", "import", "instanceof", "int", "interface", "long", "native", "new",
    "null", "package", "private", "protected", "public", "record", "return", "sealed", "short",
    "static", "super", "switch", "synchronized", "this", "throw", "throws", "transient", "true",
    "try", "var", "void", "volatile", "while", "yield",
];

const JAVA_TYPES: &[&str] = &[
    "Object", "String", "Integer", "Long", "Double", "Boolean", "List", "Map", "Set", "ByteBuffer",
];

const DART_KEYWORDS: &[&str] = &[
    "abstract", "as", "assert", "async", "await", "base", "break", "case", "catch", "class",
    "const", "continue", "covariant", "default", "deferred", "do", "dynamic", "else", "enum",
    "export", "extends", "extension", "external", "factory", "false", "final", "finally", "for",
    "Function", "get", "hide", "if", "implements", "import", "in", "interface", "is", "late",
    "library", "mixin", "new", "null", "on", "operator", "part", "required", "rethrow", "return",
    "sealed", "set", "show", "static", "super", "switch", "sync", "this", "throw", "true", "try",
    "typedef", "var", "void", "when", "while", "with", "yield",
];

const DART_TYPES: &[&str] = &[
    "int", "double", "num", "bool", "String", "List", "Map", "Set", "Future", "Stream", "Widget",
    "BuildContext", "Pointer", "Uint8List", "Float32List", "NativeFunction",
];

const TS_KEYWORDS: &[&str] = &[
    "abstract", "as", "async", "await", "break", "case", "catch", "class", "const", "continue",
    "debugger", "declare", "default", "delete", "do", "else", "enum", "export", "extends", "false",
    "finally", "for", "from", "function", "get", "if", "implements", "import", "in", "infer",
    "instanceof", "interface", "is", "keyof", "let", "module", "namespace", "new", "null", "of",
    "override", "private", "protected", "public", "readonly", "return", "satisfies", "set",
    "static", "super", "switch", "this", "throw", "true", "try", "type", "typeof", "undefined",
    "var", "void", "while", "with", "yield",
];

const TS_TYPES: &[&str] = &[
    "string", "number", "boolean", "bigint", "symbol", "unknown", "never", "any", "object",
    "Promise", "Array", "Record", "Map", "Set", "Uint8Array", "HTMLElement",
];

const SHELL_KEYWORDS: &[&str] = &[
    "if", "then", "else", "elif", "fi", "for", "in", "do", "done", "while", "until", "case",
    "esac", "function", "return", "local", "export", "unset", "readonly", "set", "shift", "exit",
    "true", "false", "cd", "echo", "printf", "test",
];

const TOML_KEYWORDS: &[&str] = &["true", "false"];
const JSON_KEYWORDS: &[&str] = &["true", "false", "null"];

const CMAKE_KEYWORDS: &[&str] = &[
    "if", "elseif", "else", "endif", "foreach", "endforeach", "while", "endwhile", "function",
    "endfunction", "macro", "endmacro", "return", "PUBLIC", "PRIVATE", "INTERFACE", "STATIC",
    "SHARED", "MODULE", "OBJECT", "REQUIRED", "QUIET", "ON", "OFF", "TRUE", "FALSE", "AND", "OR",
    "NOT", "TARGET", "TARGETS", "PROPERTIES", "COMMAND", "DEPENDS",
];
