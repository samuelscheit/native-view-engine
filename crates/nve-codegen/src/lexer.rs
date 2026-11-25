//! Lightweight zero-dependency lexer for TypeScript and Flow React Native specs.
//!
//! Handles line comments (`//`), block comments (`/* ... */`), JSDoc comments (`/** ... */`),
//! single/double/template strings, numbers (including negative/decimal), identifiers, and
//! TypeScript/Flow punctuation (`...`, `=>`, `?`, `|`, `&`, `<`, `>`, etc.).

use crate::error::CodegenError;

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    Ident(String),
    StringLit(String),
    NumberLit(String),
    DocComment(String),
    /// `...`
    Ellipsis,
    /// `=>`
    FatArrow,
    LBrace,
    RBrace,
    LParen,
    RParen,
    LBracket,
    RBracket,
    LAngle,
    RAngle,
    Colon,
    Semi,
    Comma,
    Dot,
    Question,
    Pipe,
    Amp,
    Equals,
    Plus,
    Minus,
    Star,
    Eof,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub line: usize,
    pub col: usize,
}

pub fn tokenize(src: &str) -> Result<Vec<Token>, CodegenError> {
    let bytes = src.as_bytes();
    let len = bytes.len();
    let mut pos = 0usize;
    let mut line = 1usize;
    let mut col = 1usize;
    let mut tokens = Vec::with_capacity(src.len() / 6);

    let advance = |pos: &mut usize, line: &mut usize, col: &mut usize| -> Option<char> {
        if *pos >= len {
            return None;
        }
        let ch = src[*pos..].chars().next()?;
        *pos += ch.len_utf8();
        if ch == '\n' {
            *line += 1;
            *col = 1;
        } else {
            *col += 1;
        }
        Some(ch)
    };

    while pos < len {
        let start_line = line;
        let start_col = col;
        let Some(ch) = src[pos..].chars().next() else {
            break;
        };

        // Whitespace
        if ch.is_whitespace() {
            advance(&mut pos, &mut line, &mut col);
            continue;
        }

        // Comments: `//`, `/* ... */`, `/** ... */`
        if ch == '/' && pos + 1 < len {
            let next = bytes[pos + 1];
            if next == b'/' {
                advance(&mut pos, &mut line, &mut col);
                advance(&mut pos, &mut line, &mut col);
                while let Some(c) = advance(&mut pos, &mut line, &mut col) {
                    if c == '\n' {
                        break;
                    }
                }
                continue;
            }
            if next == b'*' {
                advance(&mut pos, &mut line, &mut col);
                advance(&mut pos, &mut line, &mut col);
                let is_doc = pos < len && bytes[pos] == b'*' && (pos + 1 >= len || bytes[pos + 1] != b'/');
                if is_doc {
                    advance(&mut pos, &mut line, &mut col);
                }
                let mut raw = String::new();
                let mut closed = false;
                while pos < len {
                    if bytes[pos] == b'*' && pos + 1 < len && bytes[pos + 1] == b'/' {
                        advance(&mut pos, &mut line, &mut col);
                        advance(&mut pos, &mut line, &mut col);
                        closed = true;
                        break;
                    }
                    if let Some(c) = advance(&mut pos, &mut line, &mut col)
                        && is_doc
                    {
                        raw.push(c);
                    }
                }
                if !closed {
                    return Err(CodegenError::parse(
                        start_line,
                        start_col,
                        "unterminated block comment",
                    ));
                }
                if is_doc {
                    let cleaned = clean_doc_comment(&raw);
                    if !cleaned.is_empty() && !cleaned.contains("Copyright (c)") {
                        tokens.push(Token {
                            kind: TokenKind::DocComment(cleaned),
                            line: start_line,
                            col: start_col,
                        });
                    }
                }
                continue;
            }
        }

        // String literals ('...', "...", `...`)
        if ch == '\'' || ch == '"' || ch == '`' {
            let quote = ch;
            advance(&mut pos, &mut line, &mut col);
            let mut val = String::new();
            let mut closed = false;
            while let Some(c) = advance(&mut pos, &mut line, &mut col) {
                if c == '\\' {
                    if let Some(esc) = advance(&mut pos, &mut line, &mut col) {
                        match esc {
                            'n' => val.push('\n'),
                            'r' => val.push('\r'),
                            't' => val.push('\t'),
                            '\\' => val.push('\\'),
                            '\'' => val.push('\''),
                            '"' => val.push('"'),
                            '`' => val.push('`'),
                            other => val.push(other),
                        }
                    }
                    continue;
                }
                if c == quote {
                    closed = true;
                    break;
                }
                val.push(c);
            }
            if !closed {
                return Err(CodegenError::parse(
                    start_line,
                    start_col,
                    "unterminated string literal",
                ));
            }
            tokens.push(Token {
                kind: TokenKind::StringLit(val),
                line: start_line,
                col: start_col,
            });
            continue;
        }

        // Numbers (including hex 0x..., decimals 1.5, exponent 1e-3)
        if ch.is_ascii_digit() {
            let mut num = String::new();
            while let Some(c) = src[pos..].chars().next() {
                if c.is_ascii_alphanumeric() || c == '.' || c == '_' {
                    num.push(c);
                    advance(&mut pos, &mut line, &mut col);
                } else {
                    break;
                }
            }
            tokens.push(Token {
                kind: TokenKind::NumberLit(num),
                line: start_line,
                col: start_col,
            });
            continue;
        }

        // Identifiers (ASCII letters, `_`, `$`, plus digits after first char)
        if ch.is_ascii_alphabetic() || ch == '_' || ch == '$' {
            let mut ident = String::new();
            while let Some(c) = src[pos..].chars().next() {
                if c.is_ascii_alphanumeric() || c == '_' || c == '$' {
                    ident.push(c);
                    advance(&mut pos, &mut line, &mut col);
                } else {
                    break;
                }
            }
            tokens.push(Token {
                kind: TokenKind::Ident(ident),
                line: start_line,
                col: start_col,
            });
            continue;
        }

        // Multi-char punctuation
        if ch == '.' && pos + 2 < len && &bytes[pos..pos + 3] == b"..." {
            advance(&mut pos, &mut line, &mut col);
            advance(&mut pos, &mut line, &mut col);
            advance(&mut pos, &mut line, &mut col);
            tokens.push(Token {
                kind: TokenKind::Ellipsis,
                line: start_line,
                col: start_col,
            });
            continue;
        }
        if ch == '=' && pos + 1 < len && bytes[pos + 1] == b'>' {
            advance(&mut pos, &mut line, &mut col);
            advance(&mut pos, &mut line, &mut col);
            tokens.push(Token {
                kind: TokenKind::FatArrow,
                line: start_line,
                col: start_col,
            });
            continue;
        }

        // Single-char punctuation
        advance(&mut pos, &mut line, &mut col);
        let kind = match ch {
            '{' => TokenKind::LBrace,
            '}' => TokenKind::RBrace,
            '(' => TokenKind::LParen,
            ')' => TokenKind::RParen,
            '[' => TokenKind::LBracket,
            ']' => TokenKind::RBracket,
            '<' => TokenKind::LAngle,
            '>' => TokenKind::RAngle,
            ':' => TokenKind::Colon,
            ';' => TokenKind::Semi,
            ',' => TokenKind::Comma,
            '.' => TokenKind::Dot,
            '?' => TokenKind::Question,
            '|' => TokenKind::Pipe,
            '&' => TokenKind::Amp,
            '=' => TokenKind::Equals,
            '+' => TokenKind::Plus,
            '-' => TokenKind::Minus,
            '*' => TokenKind::Star,
            // Ignore any unrecognized punctuation outside type/spec syntax (e.g. `@`, `!`, `~`, etc.)
            _ => continue,
        };
        tokens.push(Token {
            kind,
            line: start_line,
            col: start_col,
        });
    }

    tokens.push(Token {
        kind: TokenKind::Eof,
        line,
        col,
    });
    Ok(tokens)
}

fn clean_doc_comment(raw: &str) -> String {
    let lines: Vec<String> = raw
        .lines()
        .map(|l| {
            let trimmed = l.trim();
            trimmed
                .strip_prefix("* ")
                .or_else(|| trimmed.strip_prefix('*'))
                .unwrap_or(trimmed)
                .trim_end()
                .to_owned()
        })
        .collect();

    let start = lines.iter().position(|l| !l.is_empty()).unwrap_or(lines.len());
    let end = lines
        .iter()
        .rposition(|l| !l.is_empty())
        .map(|i| i + 1)
        .unwrap_or(start);

    if start >= end {
        String::new()
    } else {
        lines[start..end].join("\n")
    }
}
