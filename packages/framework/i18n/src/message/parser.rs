//! Bounded-recursion, zero-copy-slice ICU MessageFormat parser and AST.

use crate::error::I18nError;
use crate::message::MAX_NESTING_DEPTH;
use crate::plural::{PluralCategory, PluralKind, PluralOperands};

#[derive(Debug, Clone, PartialEq)]
pub(super) enum MessageNode<'a> {
    Literal(&'a str),
    Apostrophe,
    Pound,
    Argument(&'a str),
    NumberArg {
        name: &'a str,
        style: NumberArgStyle,
    },
    Plural {
        name: &'a str,
        kind: PluralKind,
        offset: i64,
        branches: Vec<PluralBranch<'a>>,
    },
    Select {
        name: &'a str,
        branches: Vec<SelectBranch<'a>>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NumberArgStyle {
    Default,
    Integer,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct PluralBranch<'a> {
    pub selector: PluralSelector,
    pub body: Vec<MessageNode<'a>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ExactDecimalKey {
    negative: bool,
    int_part: u64,
    frac_trimmed_len: u32,
    frac_trimmed: u64,
}

impl ExactDecimalKey {
    pub(super) fn from_operands(negative: bool, op: PluralOperands) -> Self {
        let neg = negative && !(op.i == 0 && op.w == 0);
        Self {
            negative: neg,
            int_part: op.i,
            frac_trimmed_len: op.w,
            frac_trimmed: op.t,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PluralSelector {
    Exact(ExactDecimalKey),
    Category(PluralCategory),
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct SelectBranch<'a> {
    pub key: &'a str,
    pub body: Vec<MessageNode<'a>>,
}

pub(super) fn parse_template(template: &str) -> Result<Vec<MessageNode<'_>>, I18nError> {
    let mut parser = Parser::new(template);
    parser.parse_nodes(0, false, false)
}

struct Parser<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            src,
            bytes: src.as_bytes(),
            pos: 0,
        }
    }

    #[inline]
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    #[inline]
    fn peek_at(&self, delta: usize) -> Option<u8> {
        self.bytes.get(self.pos + delta).copied()
    }

    fn skip_ws(&mut self) {
        while let Some(b) = self.peek() {
            if b.is_ascii_whitespace() {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn parse_nodes(
        &mut self,
        depth: usize,
        stop_on_close_brace: bool,
        in_plural: bool,
    ) -> Result<Vec<MessageNode<'a>>, I18nError> {
        if depth > MAX_NESTING_DEPTH {
            return Err(I18nError::MaxDepthExceeded {
                max_depth: MAX_NESTING_DEPTH,
                offset: self.pos,
            });
        }

        let mut nodes = Vec::new();
        let mut lit_start = self.pos;

        while let Some(b) = self.peek() {
            match b {
                b'\'' => {
                    if self.peek_at(1) == Some(b'\'') {
                        if self.pos > lit_start {
                            nodes.push(MessageNode::Literal(&self.src[lit_start..self.pos]));
                        }
                        nodes.push(MessageNode::Apostrophe);
                        self.pos += 2;
                        lit_start = self.pos;
                    } else if matches!(self.peek_at(1), Some(b'{' | b'}'))
                        || (in_plural && self.peek_at(1) == Some(b'#'))
                    {
                        if self.pos > lit_start {
                            nodes.push(MessageNode::Literal(&self.src[lit_start..self.pos]));
                        }
                        self.parse_quoted_span(&mut nodes)?;
                        lit_start = self.pos;
                    } else {
                        self.pos += 1;
                    }
                }
                b'#' if in_plural => {
                    if self.pos > lit_start {
                        nodes.push(MessageNode::Literal(&self.src[lit_start..self.pos]));
                    }
                    nodes.push(MessageNode::Pound);
                    self.pos += 1;
                    lit_start = self.pos;
                }
                b'{' => {
                    if self.pos > lit_start {
                        nodes.push(MessageNode::Literal(&self.src[lit_start..self.pos]));
                    }
                    let node = self.parse_argument(depth)?;
                    nodes.push(node);
                    lit_start = self.pos;
                }
                b'}' => {
                    if stop_on_close_brace {
                        if self.pos > lit_start {
                            nodes.push(MessageNode::Literal(&self.src[lit_start..self.pos]));
                        }
                        return Ok(nodes);
                    }
                    return Err(I18nError::UnmatchedCloseBrace { offset: self.pos });
                }
                _ => self.pos += 1,
            }
        }

        if stop_on_close_brace {
            return Err(I18nError::UnmatchedOpenBrace { offset: self.pos });
        }

        if self.pos > lit_start {
            nodes.push(MessageNode::Literal(&self.src[lit_start..self.pos]));
        }
        Ok(nodes)
    }

    fn parse_quoted_span(&mut self, nodes: &mut Vec<MessageNode<'a>>) -> Result<(), I18nError> {
        let quote_start = self.pos;
        self.pos += 1;
        let mut seg_start = self.pos;

        while let Some(b) = self.peek() {
            if b == b'\'' {
                if self.pos > seg_start {
                    nodes.push(MessageNode::Literal(&self.src[seg_start..self.pos]));
                }
                if self.peek_at(1) == Some(b'\'') {
                    nodes.push(MessageNode::Apostrophe);
                    self.pos += 2;
                    seg_start = self.pos;
                } else {
                    self.pos += 1;
                    return Ok(());
                }
            } else {
                self.pos += 1;
            }
        }

        Err(I18nError::UnterminatedQuote {
            offset: quote_start,
        })
    }

    fn parse_argument(&mut self, depth: usize) -> Result<MessageNode<'a>, I18nError> {
        if depth >= MAX_NESTING_DEPTH {
            return Err(I18nError::MaxDepthExceeded {
                max_depth: MAX_NESTING_DEPTH,
                offset: self.pos,
            });
        }
        let open_pos = self.pos;
        self.pos += 1;
        self.skip_ws();

        let name = self.read_identifier()?;
        self.skip_ws();

        match self.peek() {
            Some(b'}') => {
                self.pos += 1;
                Ok(MessageNode::Argument(name))
            }
            Some(b',') => {
                self.pos += 1;
                self.skip_ws();
                let type_pos = self.pos;
                let arg_type = self.read_identifier()?;
                self.skip_ws();

                match arg_type {
                    "number" => self.parse_number_arg(name, open_pos),
                    "plural" => {
                        self.expect_byte(b',', "',' after plural type")?;
                        self.parse_plural_arg(name, PluralKind::Cardinal, depth + 1, open_pos)
                    }
                    "selectordinal" => {
                        self.expect_byte(b',', "',' after selectordinal type")?;
                        self.parse_plural_arg(name, PluralKind::Ordinal, depth + 1, open_pos)
                    }
                    "select" => {
                        self.expect_byte(b',', "',' after select type")?;
                        self.parse_select_arg(name, depth + 1, open_pos)
                    }
                    other => Err(I18nError::InvalidArgumentType {
                        arg_type: other.to_owned(),
                        offset: type_pos,
                    }),
                }
            }
            None => Err(I18nError::UnmatchedOpenBrace { offset: open_pos }),
            Some(_) => Err(I18nError::ExpectedToken {
                expected: "',' or '}'",
                offset: self.pos,
            }),
        }
    }

    fn parse_number_arg(
        &mut self,
        name: &'a str,
        open_pos: usize,
    ) -> Result<MessageNode<'a>, I18nError> {
        let style = match self.peek() {
            Some(b'}') => {
                self.pos += 1;
                NumberArgStyle::Default
            }
            Some(b',') => {
                self.pos += 1;
                self.skip_ws();
                let style_pos = self.pos;
                let style_name = self.read_identifier()?;
                self.skip_ws();
                self.expect_byte(b'}', "'}' after number style")?;
                match style_name {
                    "integer" => NumberArgStyle::Integer,
                    other => {
                        return Err(I18nError::InvalidArgumentType {
                            arg_type: format!("number,{other}"),
                            offset: style_pos,
                        });
                    }
                }
            }
            None => return Err(I18nError::UnmatchedOpenBrace { offset: open_pos }),
            Some(_) => {
                return Err(I18nError::ExpectedToken {
                    expected: "',' or '}'",
                    offset: self.pos,
                });
            }
        };
        Ok(MessageNode::NumberArg { name, style })
    }

    fn parse_plural_arg(
        &mut self,
        name: &'a str,
        kind: PluralKind,
        depth: usize,
        open_pos: usize,
    ) -> Result<MessageNode<'a>, I18nError> {
        let mut offset_val: i64 = 0;
        let mut branches: Vec<PluralBranch<'a>> = Vec::new();
        let mut has_other = false;

        self.skip_ws();
        if self.src[self.pos..].starts_with("offset:") {
            let off_start = self.pos;
            self.pos += "offset:".len();
            let num_start = self.pos;
            while let Some(b) = self.peek() {
                if b.is_ascii_digit() {
                    self.pos += 1;
                } else {
                    break;
                }
            }
            if self.pos == num_start {
                return Err(I18nError::InvalidOffset { offset: off_start });
            }
            offset_val = self.src[num_start..self.pos]
                .parse::<i64>()
                .map_err(|_| I18nError::InvalidOffset { offset: off_start })?;
        }

        loop {
            self.skip_ws();
            match self.peek() {
                Some(b'}') => {
                    self.pos += 1;
                    break;
                }
                None => return Err(I18nError::UnmatchedOpenBrace { offset: open_pos }),
                Some(_) => {}
            }

            let sel_pos = self.pos;
            let raw_sel = self.read_selector_token()?;
            let selector = parse_plural_selector(raw_sel, sel_pos)?;

            if branches.iter().any(|b| b.selector == selector) {
                return Err(I18nError::DuplicateSelector {
                    selector: raw_sel.to_owned(),
                    offset: sel_pos,
                });
            }
            if selector == PluralSelector::Category(PluralCategory::Other) {
                has_other = true;
            }

            self.skip_ws();
            self.expect_byte(b'{', "'{' starting plural branch")?;
            let body = self.parse_nodes(depth, true, true)?;
            self.expect_byte(b'}', "'}' closing plural branch")?;

            branches.push(PluralBranch { selector, body });
        }

        if !has_other {
            return Err(I18nError::MissingOtherBranch {
                arg: name.to_owned(),
                offset: open_pos,
            });
        }

        Ok(MessageNode::Plural {
            name,
            kind,
            offset: offset_val,
            branches,
        })
    }

    fn parse_select_arg(
        &mut self,
        name: &'a str,
        depth: usize,
        open_pos: usize,
    ) -> Result<MessageNode<'a>, I18nError> {
        let mut branches: Vec<SelectBranch<'a>> = Vec::new();
        let mut has_other = false;

        loop {
            self.skip_ws();
            match self.peek() {
                Some(b'}') => {
                    self.pos += 1;
                    break;
                }
                None => return Err(I18nError::UnmatchedOpenBrace { offset: open_pos }),
                Some(_) => {}
            }

            let key_pos = self.pos;
            let key = self.read_selector_token()?;
            if branches.iter().any(|b| b.key == key) {
                return Err(I18nError::DuplicateSelector {
                    selector: key.to_owned(),
                    offset: key_pos,
                });
            }
            if key == "other" {
                has_other = true;
            }

            self.skip_ws();
            self.expect_byte(b'{', "'{' starting select branch")?;
            let body = self.parse_nodes(depth, true, false)?;
            self.expect_byte(b'}', "'}' closing select branch")?;

            branches.push(SelectBranch { key, body });
        }

        if !has_other {
            return Err(I18nError::MissingOtherBranch {
                arg: name.to_owned(),
                offset: open_pos,
            });
        }

        Ok(MessageNode::Select { name, branches })
    }

    fn read_identifier(&mut self) -> Result<&'a str, I18nError> {
        let start = self.pos;
        while let Some(b) = self.peek() {
            if b.is_ascii_alphanumeric() || b == b'_' || b == b'-' {
                self.pos += 1;
            } else {
                break;
            }
        }
        if self.pos == start {
            return Err(I18nError::EmptyArgumentName { offset: start });
        }
        Ok(&self.src[start..self.pos])
    }

    fn read_selector_token(&mut self) -> Result<&'a str, I18nError> {
        let start = self.pos;
        while let Some(b) = self.peek() {
            if b.is_ascii_whitespace() || b == b'{' || b == b'}' || b == b',' {
                break;
            }
            self.pos += 1;
        }
        if self.pos == start {
            return Err(I18nError::ExpectedToken {
                expected: "selector keyword",
                offset: start,
            });
        }
        Ok(&self.src[start..self.pos])
    }

    fn expect_byte(&mut self, expected: u8, desc: &'static str) -> Result<(), I18nError> {
        if self.peek() == Some(expected) {
            self.pos += 1;
            Ok(())
        } else {
            Err(I18nError::ExpectedToken {
                expected: desc,
                offset: self.pos,
            })
        }
    }
}

fn parse_plural_selector(raw: &str, offset: usize) -> Result<PluralSelector, I18nError> {
    if let Some(num_str) = raw.strip_prefix('=') {
        if num_str.is_empty() {
            return Err(I18nError::InvalidPluralSelector {
                selector: raw.to_owned(),
                offset,
            });
        }
        let negative = num_str.starts_with('-');
        let op = PluralOperands::parse(num_str).map_err(|_| I18nError::InvalidPluralSelector {
            selector: raw.to_owned(),
            offset,
        })?;
        return Ok(PluralSelector::Exact(ExactDecimalKey::from_operands(
            negative, op,
        )));
    }

    PluralCategory::parse(raw)
        .map(PluralSelector::Category)
        .ok_or_else(|| I18nError::InvalidPluralSelector {
            selector: raw.to_owned(),
            offset,
        })
}
