//! Built-in pseudo-localization for CI & UI QA layout, truncation, and encoding checks.
//!
//! [`pseudo_localize`] transforms UI copy by:
//! 1. Wrapping the outer string in `[...]` delimiters to expose clipping/truncation.
//! 2. Replacing ASCII letters with accented Unicode lookalikes to catch non-UTF-8
//!    handling and font fallback defects.
//! 3. Expanding visible text length by ~35–40% to simulate German/Finnish text
//!    expansion and catch fixed-width layout overflows.
//! 4. Preserving ICU `{placeholders}`, `plural`/`selectordinal`/`select` headers,
//!    selectors, `#`, and apostrophe escapes intact so pseudo-localized ICU
//!    templates remain valid and formattable.

use crate::message::MAX_NESTING_DEPTH;

/// Configuration for [`PseudoLocalizer`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PseudoLocaleOptions {
    /// Percentage length expansion applied to visible literal text (default: `38`, i.e. 38%).
    pub expansion_percent: u32,
    /// Opening delimiter wrapped around the final string (default: `"["`).
    pub prefix: &'static str,
    /// Closing delimiter wrapped around the final string (default: `"]"`).
    pub suffix: &'static str,
    /// Whether to replace ASCII letters with accented lookalikes (default: `true`).
    pub accent_ascii: bool,
}

impl Default for PseudoLocaleOptions {
    fn default() -> Self {
        Self {
            expansion_percent: 38,
            prefix: "[",
            suffix: "]",
            accent_ascii: true,
        }
    }
}

/// Configurable pseudo-localizer preserving ICU MessageFormat syntax.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PseudoLocalizer {
    options: PseudoLocaleOptions,
}

impl PseudoLocalizer {
    /// Creates a new [`PseudoLocalizer`] with custom options.
    pub const fn new(options: PseudoLocaleOptions) -> Self {
        Self { options }
    }

    /// Returns the active [`PseudoLocaleOptions`].
    #[inline]
    pub const fn options(&self) -> PseudoLocaleOptions {
        self.options
    }

    /// Pseudo-localizes `input`, preserving ICU placeholders and syntax.
    pub fn transform(&self, input: &str) -> String {
        let mut out = String::with_capacity(input.len() * 2 + 4);
        out.push_str(self.options.prefix);
        self.transform_segment(input, false, 0, &mut out);
        out.push_str(self.options.suffix);
        out
    }

    fn transform_segment(&self, input: &str, in_plural: bool, depth: usize, out: &mut String) {
        let bytes = input.as_bytes();
        let mut pos = 0usize;
        let mut lit_start = 0usize;

        while pos < bytes.len() {
            match bytes[pos] {
                b'\'' => {
                    if bytes.get(pos + 1) == Some(&b'\'') {
                        self.emit_literal(&input[lit_start..pos], out);
                        out.push_str("''");
                        pos += 2;
                        lit_start = pos;
                    } else if matches!(bytes.get(pos + 1), Some(b'{' | b'}'))
                        || (in_plural && bytes.get(pos + 1) == Some(&b'#'))
                    {
                        self.emit_literal(&input[lit_start..pos], out);
                        let quote_start = pos;
                        pos += 1;
                        while pos < bytes.len() {
                            if bytes[pos] == b'\'' {
                                if bytes.get(pos + 1) == Some(&b'\'') {
                                    pos += 2;
                                } else {
                                    pos += 1;
                                    break;
                                }
                            } else {
                                pos += 1;
                            }
                        }
                        out.push_str(&input[quote_start..pos]);
                        lit_start = pos;
                    } else {
                        pos += 1;
                    }
                }
                b'#' if in_plural => {
                    self.emit_literal(&input[lit_start..pos], out);
                    out.push('#');
                    pos += 1;
                    lit_start = pos;
                }
                b'{' if depth < MAX_NESTING_DEPTH => {
                    if let Some(end) = find_matching_brace(input, pos) {
                        self.emit_literal(&input[lit_start..pos], out);
                        self.transform_brace_block(&input[pos..=end], depth + 1, out);
                        pos = end + 1;
                        lit_start = pos;
                    } else {
                        pos += 1;
                    }
                }
                _ => pos += 1,
            }
        }

        self.emit_literal(&input[lit_start..], out);
    }

    fn transform_brace_block(&self, block: &str, depth: usize, out: &mut String) {
        let inner = &block[1..block.len() - 1];
        let Some((arg_name, rest)) = inner.split_once(',') else {
            // Simple `{name}` placeholder — preserve completely intact.
            out.push_str(block);
            return;
        };

        let Some((arg_type_raw, body_raw)) = rest.split_once(',') else {
            // Two-part placeholder such as `{amount, number}` — preserve intact.
            out.push_str(block);
            return;
        };

        let arg_type = arg_type_raw.trim();
        let is_plural = matches!(arg_type, "plural" | "selectordinal");
        let is_select = arg_type == "select";
        if !is_plural && !is_select {
            // `{amount, number, integer}` etc. — preserve intact.
            out.push_str(block);
            return;
        }

        out.push('{');
        out.push_str(arg_name);
        out.push(',');
        out.push_str(arg_type_raw);
        out.push(',');

        // Walk `offset:N` and `selector { submessage }` segments inside `body_raw`.
        let b = body_raw.as_bytes();
        let mut idx = 0usize;
        while idx < b.len() {
            if b[idx] == b'{' {
                if let Some(close) = find_matching_brace(body_raw, idx) {
                    out.push('{');
                    self.transform_segment(&body_raw[idx + 1..close], is_plural, depth, out);
                    out.push('}');
                    idx = close + 1;
                    continue;
                }
                out.push_str(&body_raw[idx..]);
                return;
            }
            let ch = body_raw[idx..].chars().next().unwrap_or(' ');
            out.push(ch);
            idx += ch.len_utf8();
        }
        out.push('}');
    }

    fn emit_literal(&self, text: &str, out: &mut String) {
        if text.is_empty() {
            return;
        }

        let visible_count = text.chars().count();
        let letter_count = text.chars().filter(|c| c.is_alphabetic()).count();
        if letter_count == 0 {
            out.push_str(text);
            return;
        }

        let target_extra = if self.options.expansion_percent == 0 {
            0
        } else {
            let calc = (visible_count * self.options.expansion_percent as usize + 50) / 100;
            if visible_count >= 2 { calc.max(1) } else { calc }
        };

        // Count vowels eligible for natural elongation.
        let vowel_count = text.chars().filter(|c| is_ascii_vowel(*c)).count();
        let mut remaining = target_extra;

        if vowel_count > 0 && remaining > 0 {
            let base_repeat = remaining / vowel_count;
            let mut extra_ones = remaining % vowel_count;

            for ch in text.chars() {
                let mapped = self.map_char(ch);
                out.push(mapped);
                if is_ascii_vowel(ch) && remaining > 0 {
                    let reps = base_repeat + usize::from(extra_ones > 0);
                    extra_ones = extra_ones.saturating_sub(1);
                    let actual = reps.min(remaining);
                    for _ in 0..actual {
                        out.push(mapped);
                    }
                    remaining -= actual;
                }
            }
        } else {
            for ch in text.chars() {
                out.push(self.map_char(ch));
            }
        }

        if remaining > 0 {
            const PAD: [char; 5] = ['ȧ', 'ḗ', 'ī', 'ǿ', 'ŭ'];
            for idx in 0..remaining {
                out.push(if self.options.accent_ascii {
                    PAD[idx % PAD.len()]
                } else {
                    '~'
                });
            }
        }
    }

    #[inline]
    fn map_char(&self, ch: char) -> char {
        if self.options.accent_ascii {
            accent_ascii_char(ch)
        } else {
            ch
        }
    }
}

/// Pseudo-localizes `input` with default options (`[...]` delimiters, accented
/// ASCII lookalikes, ~38% expansion, ICU placeholders preserved).
pub fn pseudo_localize(input: &str) -> String {
    PseudoLocalizer::default().transform(input)
}

/// Pseudo-localizes `input` with explicit [`PseudoLocaleOptions`].
pub fn pseudo_localize_with_options(input: &str, opts: PseudoLocaleOptions) -> String {
    PseudoLocalizer::new(opts).transform(input)
}

fn find_matching_brace(s: &str, open_idx: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut depth = 0usize;
    let mut pos = open_idx;
    let mut in_quote = false;

    while pos < bytes.len() {
        let b = bytes[pos];
        if in_quote {
            if b == b'\'' {
                if bytes.get(pos + 1) == Some(&b'\'') {
                    pos += 2;
                    continue;
                }
                in_quote = false;
            }
            pos += 1;
            continue;
        }

        match b {
            b'\'' => {
                if bytes.get(pos + 1) == Some(&b'\'') {
                    pos += 2;
                    continue;
                }
                if matches!(bytes.get(pos + 1), Some(b'{' | b'}' | b'#')) {
                    in_quote = true;
                }
            }
            b'{' => depth += 1,
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(pos);
                }
            }
            _ => {}
        }
        pos += 1;
    }
    None
}

#[inline]
fn is_ascii_vowel(ch: char) -> bool {
    matches!(ch, 'a' | 'e' | 'i' | 'o' | 'u' | 'A' | 'E' | 'I' | 'O' | 'U')
}

#[inline]
fn accent_ascii_char(ch: char) -> char {
    match ch {
        'a' => 'ȧ',
        'b' => 'ƀ',
        'c' => 'ƈ',
        'd' => 'ḓ',
        'e' => 'ḗ',
        'f' => 'ƒ',
        'g' => 'ɠ',
        'h' => 'ħ',
        'i' => 'ī',
        'j' => 'ĵ',
        'k' => 'ķ',
        'l' => 'ŀ',
        'm' => 'ḿ',
        'n' => 'ƞ',
        'o' => 'ǿ',
        'p' => 'ƥ',
        'q' => 'ɋ',
        'r' => 'ř',
        's' => 'ş',
        't' => 'ŧ',
        'u' => 'ŭ',
        'v' => 'ṽ',
        'w' => 'ẇ',
        'x' => 'ẋ',
        'y' => 'ẏ',
        'z' => 'ẑ',
        'A' => 'Ȧ',
        'B' => 'Ɓ',
        'C' => 'Ƈ',
        'D' => 'Ḓ',
        'E' => 'Ḗ',
        'F' => 'Ƒ',
        'G' => 'Ɠ',
        'H' => 'Ħ',
        'I' => 'Ī',
        'J' => 'Ĵ',
        'K' => 'Ķ',
        'L' => 'Ŀ',
        'M' => 'Ḿ',
        'N' => 'Ƞ',
        'O' => 'Ǿ',
        'P' => 'Ƥ',
        'Q' => 'Ɋ',
        'R' => 'Ř',
        'S' => 'Ş',
        'T' => 'Ŧ',
        'U' => 'Ŭ',
        'V' => 'Ṽ',
        'W' => 'Ẇ',
        'X' => 'Ẋ',
        'Y' => 'Ẏ',
        'Z' => 'Ẑ',
        other => other,
    }
}
