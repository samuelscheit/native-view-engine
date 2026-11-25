use crate::error::CodegenError;

#[derive(Debug, Clone, PartialEq)]
pub(super) enum TokKind {
    Ident(String),
    Int(u64, bool),
    Float(f64),
    Str(String),
    DocComment(String),
    Hash,
    Bang,
    LBracket,
    RBracket,
    LParen,
    RParen,
    LBrace,
    RBrace,
    LAngle,
    RAngle,
    Comma,
    Colon,
    ColonColon,
    Semi,
    Eq,
    Arrow,
    Amp,
    Minus,
    Star,
    Dot,
    Eof,
}

#[derive(Debug, Clone)]
pub(super) struct Tok {
    pub(super) kind: TokKind,
    pub(super) line: usize,
    pub(super) col: usize,
}

pub(super) fn tokenize(src: &str) -> Result<Vec<Tok>, CodegenError> {
    let chars: Vec<char> = src.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0usize;
    let mut line = 1usize;
    let mut col = 1usize;

    let advance = |i: &mut usize, line: &mut usize, col: &mut usize| -> Option<char> {
        let c = chars.get(*i).copied()?;
        *i += 1;
        if c == '\n' {
            *line += 1;
            *col = 1;
        } else {
            *col += 1;
        }
        Some(c)
    };

    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        let next2 = chars.get(i + 2).copied();

        if c.is_whitespace() {
            advance(&mut i, &mut line, &mut col);
            continue;
        }

        // Line comments & doc comments (`///` or `//!`)
        if c == '/' && next == Some('/') {
            let start_line = line;
            let start_col = col;
            let is_doc = next2 == Some('/') && chars.get(i + 3).copied() != Some('/');
            advance(&mut i, &mut line, &mut col);
            advance(&mut i, &mut line, &mut col);
            if is_doc {
                advance(&mut i, &mut line, &mut col);
            }
            let mut text = String::new();
            while i < chars.len() && chars[i] != '\n' {
                if let Some(ch) = advance(&mut i, &mut line, &mut col) {
                    text.push(ch);
                }
            }
            if is_doc {
                tokens.push(Tok {
                    kind: TokKind::DocComment(text.trim().to_string()),
                    line: start_line,
                    col: start_col,
                });
            }
            continue;
        }

        // Block comments (supports nesting per Rust lexical rules)
        if c == '/' && next == Some('*') {
            let start_line = line;
            let start_col = col;
            advance(&mut i, &mut line, &mut col);
            advance(&mut i, &mut line, &mut col);
            let mut depth = 1usize;
            while i < chars.len() && depth > 0 {
                if chars[i] == '/' && chars.get(i + 1).copied() == Some('*') {
                    depth += 1;
                    advance(&mut i, &mut line, &mut col);
                    advance(&mut i, &mut line, &mut col);
                } else if chars[i] == '*' && chars.get(i + 1).copied() == Some('/') {
                    depth -= 1;
                    advance(&mut i, &mut line, &mut col);
                    advance(&mut i, &mut line, &mut col);
                } else {
                    advance(&mut i, &mut line, &mut col);
                }
            }
            if depth > 0 {
                return Err(CodegenError::parse(
                    start_line,
                    start_col,
                    "unterminated block comment",
                ));
            }
            continue;
        }

        let start_line = line;
        let start_col = col;

        // String literal
        if c == '"' {
            advance(&mut i, &mut line, &mut col);
            let mut out = String::new();
            let mut closed = false;
            while let Some(ch) = advance(&mut i, &mut line, &mut col) {
                if ch == '"' {
                    closed = true;
                    break;
                }
                if ch == '\\' {
                    let esc = advance(&mut i, &mut line, &mut col).ok_or_else(|| {
                        CodegenError::parse(start_line, start_col, "unterminated string escape")
                    })?;
                    match esc {
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        '\\' => out.push('\\'),
                        '"' => out.push('"'),
                        '0' => out.push('\0'),
                        other => {
                            return Err(CodegenError::parse(
                                line,
                                col,
                                format!("unsupported string escape '\\{other}'"),
                            ));
                        }
                    }
                } else {
                    out.push(ch);
                }
            }
            if !closed {
                return Err(CodegenError::parse(
                    start_line,
                    start_col,
                    "unterminated string literal",
                ));
            }
            tokens.push(Tok {
                kind: TokKind::Str(out),
                line: start_line,
                col: start_col,
            });
            continue;
        }

        // Lifetimes (`'a`, `'static`) — skip `'ident` smoothly for references like `&'a str`
        if c == '\'' {
            advance(&mut i, &mut line, &mut col);
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                advance(&mut i, &mut line, &mut col);
            }
            continue;
        }

        // Numbers (decimal / hex / underscores / float)
        if c.is_ascii_digit() {
            let mut raw = String::new();
            if c == '0' && matches!(next, Some('x') | Some('X')) {
                advance(&mut i, &mut line, &mut col);
                advance(&mut i, &mut line, &mut col);
                while i < chars.len() && (chars[i].is_ascii_hexdigit() || chars[i] == '_') {
                    if let Some(ch) = advance(&mut i, &mut line, &mut col)
                        && ch != '_'
                    {
                        raw.push(ch);
                    }
                }
                let val = u64::from_str_radix(&raw, 16).map_err(|_| {
                    CodegenError::parse(
                        start_line,
                        start_col,
                        format!("invalid hex integer literal '0x{raw}'"),
                    )
                })?;
                tokens.push(Tok {
                    kind: TokKind::Int(val, true),
                    line: start_line,
                    col: start_col,
                });
                continue;
            }
            let mut is_float = false;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '_') {
                if let Some(ch) = advance(&mut i, &mut line, &mut col)
                    && ch != '_'
                {
                    raw.push(ch);
                }
            }
            if i < chars.len()
                && chars[i] == '.'
                && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit())
            {
                is_float = true;
                if let Some(dot) = advance(&mut i, &mut line, &mut col) {
                    raw.push(dot);
                }
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '_') {
                    if let Some(ch) = advance(&mut i, &mut line, &mut col)
                        && ch != '_'
                    {
                        raw.push(ch);
                    }
                }
            }
            if i < chars.len() && matches!(chars[i], 'e' | 'E') {
                is_float = true;
                if let Some(exp) = advance(&mut i, &mut line, &mut col) {
                    raw.push(exp);
                }
                if i < chars.len() && matches!(chars[i], '+' | '-')
                    && let Some(sign) = advance(&mut i, &mut line, &mut col)
                {
                    raw.push(sign);
                }
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '_') {
                    if let Some(ch) = advance(&mut i, &mut line, &mut col)
                        && ch != '_'
                    {
                        raw.push(ch);
                    }
                }
            }
            // Skip optional Rust numeric suffix (`u32`, `i64`, `f32`, `f64`, etc.)
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                advance(&mut i, &mut line, &mut col);
            }
            if is_float {
                let val = raw.parse::<f64>().map_err(|_| {
                    CodegenError::parse(
                        start_line,
                        start_col,
                        format!("invalid float literal '{raw}'"),
                    )
                })?;
                tokens.push(Tok {
                    kind: TokKind::Float(val),
                    line: start_line,
                    col: start_col,
                });
            } else {
                let val = raw.parse::<u64>().map_err(|_| {
                    CodegenError::parse(
                        start_line,
                        start_col,
                        format!("invalid integer literal '{raw}'"),
                    )
                })?;
                tokens.push(Tok {
                    kind: TokKind::Int(val, false),
                    line: start_line,
                    col: start_col,
                });
            }
            continue;
        }

        // Identifiers
        if c.is_ascii_alphabetic() || c == '_' {
            let mut ident = String::new();
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                if let Some(ch) = advance(&mut i, &mut line, &mut col) {
                    ident.push(ch);
                }
            }
            tokens.push(Tok {
                kind: TokKind::Ident(ident),
                line: start_line,
                col: start_col,
            });
            continue;
        }

        // Two-character punctuation
        if c == ':' && next == Some(':') {
            advance(&mut i, &mut line, &mut col);
            advance(&mut i, &mut line, &mut col);
            tokens.push(Tok {
                kind: TokKind::ColonColon,
                line: start_line,
                col: start_col,
            });
            continue;
        }
        if c == '-' && next == Some('>') {
            advance(&mut i, &mut line, &mut col);
            advance(&mut i, &mut line, &mut col);
            tokens.push(Tok {
                kind: TokKind::Arrow,
                line: start_line,
                col: start_col,
            });
            continue;
        }

        // Single-character punctuation
        advance(&mut i, &mut line, &mut col);
        let kind = match c {
            '#' => TokKind::Hash,
            '!' => TokKind::Bang,
            '[' => TokKind::LBracket,
            ']' => TokKind::RBracket,
            '(' => TokKind::LParen,
            ')' => TokKind::RParen,
            '{' => TokKind::LBrace,
            '}' => TokKind::RBrace,
            '<' => TokKind::LAngle,
            '>' => TokKind::RAngle,
            ',' => TokKind::Comma,
            ':' => TokKind::Colon,
            ';' => TokKind::Semi,
            '=' => TokKind::Eq,
            '&' => TokKind::Amp,
            '-' => TokKind::Minus,
            '*' => TokKind::Star,
            '.' => TokKind::Dot,
            other => {
                return Err(CodegenError::parse(
                    start_line,
                    start_col,
                    format!("unexpected character '{other}' in hybrid spec"),
                ));
            }
        };
        tokens.push(Tok {
            kind,
            line: start_line,
            col: start_col,
        });
    }

    tokens.push(Tok {
        kind: TokKind::Eof,
        line,
        col,
    });
    Ok(tokens)
}

