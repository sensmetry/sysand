// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Module for extracting symbols from KerML or SysML files.
//!
//! The main function is `top_level`, which returns top level symbols for a
//! given file. We need to know the top level symbols to correctly populate
//! field `index` of `.meta.json` files.

mod lex;

use std::{collections::HashMap, io, iter::Peekable};

use logos::{Logos, Source};
use thiserror::Error;

use lex::Token;
use typed_path::Utf8UnixPath;

use crate::symbols::lex::LexingError;

#[derive(Debug)]
pub enum Language {
    SysML,
    KerML,
}

impl Language {
    pub fn from_suffix<S: AsRef<str>>(suffix: S) -> Option<Language> {
        match suffix.as_ref().to_ascii_lowercase().as_str() {
            "sysml" => Some(Language::SysML),
            "kerml" => Some(Language::KerML),
            _ => None,
        }
    }

    pub fn guess_from_path<P: AsRef<Utf8UnixPath>>(path: P) -> Option<Language> {
        path.as_ref().extension().and_then(Language::from_suffix)
    }
}

fn parse_name<'a, I: Iterator<Item = &'a (Token, Box<str>, logos::Span)>>(
    token_iter: &mut Peekable<I>,
) -> Result<String, ParseError> {
    match token_iter.next() {
        None => Err(ParseError {
            span: None,
            msg: "empty name".to_string(),
        }),
        Some((Token::Quoted, original, _)) => Ok(original.trim_matches('\'').to_string()),
        Some((Token::OtherIdentifier, original, _)) => Ok(original.to_string()),
        Some((invalid_token, original, sp)) => Err(ParseError {
            span: Some(sp.clone()),
            msg: format!(
                "invalid token of type {:?}, expected a name component: '{}'",
                invalid_token, original
            ),
        }),
    }
}

fn parse_reference_or_chain<'a, I: Iterator<Item = &'a (Token, Box<str>, logos::Span)>>(
    token_iter: &mut Peekable<I>,
) -> Result<Vec<String>, ParseError> {
    let mut result = vec![];

    loop {
        let name_component = parse_name(token_iter)?;

        result.push(name_component);

        match token_iter.peek() {
            Some((Token::DoubleColon, ..)) | Some((Token::Period, ..)) => {
                token_iter.next();
            }
            _ => {
                break;
            }
        }
    }

    Ok(result)
}

fn skip_nested<'a, I: Iterator<Item = &'a (Token, Box<str>, logos::Span)>>(
    token_iter: &mut Peekable<I>,
    open: Token,
    close: Token,
) -> Result<(), ParseError> {
    let mut nesting = 0;

    loop {
        match token_iter.next() {
            Some((token, ..)) => {
                if token == &open {
                    nesting += 1;
                } else if token == &close {
                    nesting -= 1;
                    if nesting == 0 {
                        break;
                    }
                }
            }
            None => {
                return Err(ParseError {
                    span: None,
                    msg: format!("unmatched {:?}", open),
                });
            }
        }
    }

    Ok(())
}

fn skip_whitespace<'a, I: Iterator<Item = &'a (Token, Box<str>, logos::Span)>>(
    token_iter: &mut Peekable<I>,
) {
    while let Some((Token::Space, ..)) = token_iter.peek() {
        token_iter.next();
    }
}

fn maybe_skip_muliplicity<'a, I: Iterator<Item = &'a (Token, Box<str>, logos::Span)>>(
    token_iter: &mut Peekable<I>,
) -> Result<(), ParseError> {
    if let Some((Token::OpenSquare, ..)) = token_iter.peek() {
        skip_nested(token_iter, Token::OpenSquare, Token::CloseSquare)?;
    };

    Ok(())
}

fn skip_list_of_refs<'a, I: Iterator<Item = &'a (Token, Box<str>, logos::Span)>>(
    token_iter: &mut Peekable<I>,
) -> Result<(), ParseError> {
    loop {
        let _ = parse_reference_or_chain(token_iter)?;

        skip_whitespace(token_iter);

        match token_iter.peek() {
            Some((Token::Comma, ..)) => {
                token_iter.next();
                skip_whitespace(token_iter);
            }
            _ => {
                return Ok(());
            }
        }
    }
}

enum KeywordType {
    Simple,
    MultiReferences,
    MultiReferencesWithMultiplicity,
    SkipRest,
}

struct ParseError {
    span: Option<logos::Span>,
    msg: String,
}

fn parse_entity<'a, I: Iterator<Item = &'a (Token, Box<str>, logos::Span)>>(
    token_iter: &'a mut Peekable<I>,
    keywords: &HashMap<&str, KeywordType>,
) -> Result<(Option<String>, Option<String>), ParseError> {
    let mut long_name = None;
    let mut short_name = None;

    while let Some((token, original, sp)) = token_iter.peek() {
        match token {
            Token::Space => {
                token_iter.next();
            }
            Token::UserKeywordStart => {
                token_iter.next();
                parse_reference_or_chain(token_iter)?;
            }
            Token::BraceOpen => unreachable!(),
            Token::BraceClose => unreachable!(),
            Token::Semicolon => unreachable!(),
            Token::Comma => {
                return Err(ParseError {
                    span: Some(sp.clone()),
                    msg: "floating comma".to_string(),
                });
            }
            Token::OtherIdentifier | Token::Quoted => {
                match keywords.get(&**original) {
                    Some(KeywordType::Simple) => {
                        token_iter.next();
                        continue;
                    }
                    Some(KeywordType::MultiReferences) => {
                        token_iter.next();
                        skip_whitespace(token_iter);
                        skip_list_of_refs(token_iter)?;
                        continue;
                    }
                    Some(KeywordType::MultiReferencesWithMultiplicity) => {
                        token_iter.next();
                        skip_whitespace(token_iter);
                        maybe_skip_muliplicity(token_iter)?;
                        skip_whitespace(token_iter);
                        skip_list_of_refs(token_iter)?;
                        continue;
                    }
                    Some(KeywordType::SkipRest) => {
                        break;
                    }
                    None => {
                        // Try to parse as name
                        let this_ref = parse_reference_or_chain(token_iter)?;

                        let this_name = match this_ref.as_slice() {
                            [name] => name,
                            names => {
                                return Err(ParseError {
                                    span: None,
                                    msg: format!("warn: got a floating reference: {:?}", names),
                                });
                            }
                        };

                        match long_name {
                            None => {
                                long_name = Some(this_name.clone());
                            }
                            Some(_) => {
                                return Err(ParseError {
                                    span: Some(sp.clone()),
                                    msg: format!("unknown name '{}'", this_name),
                                });
                            }
                        };
                    }
                }
            }
            Token::BlockComment => {
                token_iter.next();
            }
            Token::LineComment => {
                token_iter.next();
            }
            Token::String => panic!("floating string"),
            Token::OpenParen => skip_nested(token_iter, Token::OpenParen, Token::CloseParen)?,
            Token::CloseParen => panic!("floating closing paren"),
            Token::OpenSquare => skip_nested(token_iter, Token::OpenSquare, Token::CloseSquare)?,
            Token::CloseSquare => panic!("floating closing square bracket"),
            Token::DoubleColon => panic!("floating double colon"),
            Token::LT => {
                token_iter.next();
                let this_name = parse_name(token_iter)?;
                match short_name {
                    None => {
                        short_name = Some(this_name);
                    }
                    Some(_) => {
                        return Err(ParseError {
                            span: Some(sp.clone()),
                            msg: format!("unknown name '{}'", this_name),
                        });
                    }
                };
                match token_iter.next() {
                    None => {
                        return Err(ParseError {
                            span: Some(sp.clone()),
                            msg: "unclosed short-name '<'".to_string(),
                        });
                    }
                    Some((Token::GT, ..)) => {}
                    Some((invalid_token, original, span)) => {
                        return Err(ParseError {
                            span: Some(sp.start..span.end),
                            msg: format!(
                                "expected '<', found '{}' (token {:?})",
                                original, invalid_token
                            ),
                        });
                    }
                };
            }
            Token::GT => {
                return Err(ParseError {
                    span: Some(sp.clone()),
                    msg: "floating '<'".to_string(),
                });
            }
            Token::Equals => {
                break;
            }
            Token::Period => {
                return Err(ParseError {
                    span: Some(sp.clone()),
                    msg: "floating period".to_string(),
                });
            }
            Token::OtherSymbol => match keywords.get(&**original) {
                Some(KeywordType::Simple) => {
                    token_iter.next();
                    continue;
                }
                Some(KeywordType::MultiReferences) => {
                    token_iter.next();
                    skip_whitespace(token_iter);
                    skip_list_of_refs(token_iter)?;
                    continue;
                }
                Some(KeywordType::MultiReferencesWithMultiplicity) => {
                    token_iter.next();
                    skip_whitespace(token_iter);
                    maybe_skip_muliplicity(token_iter)?;
                    skip_whitespace(token_iter);
                    skip_list_of_refs(token_iter)?;
                    continue;
                }
                Some(KeywordType::SkipRest) => {
                    break;
                }
                None => {
                    return Err(ParseError {
                        span: Some(sp.clone()),
                        msg: format!("floating unknown symbol '{}'", original),
                    });
                }
            },
        };
    }

    Ok((long_name, short_name))
}

#[derive(Debug, Error)]
pub enum ExtractError {
    #[error("failed to read file to extract symbols: {0}")]
    ReadTopLevelSysml(io::Error),
    #[error("failed to read file to extract symbols: {0}")]
    ReadTopLevelKerml(io::Error),
    #[error("syntax error at line {0}, byte {1}:\n{2}")]
    Syntax(u32, u32, LexingError),
    #[error(
        "missing body delimiter: brace '{{}}' nesting depth is {0} (should be 0) at the end of file"
    )]
    MissingBodyDelimiter(i32),
    #[error("unable to get token range")]
    TokenRange,
    #[error("error at line {0}, byte {1}:\n'{2}': {3}")]
    Parse(u32, u32, String, String),
}

fn read_source<R: io::Read>(
    mut reader: R,
    error_constructor: impl FnOnce(io::Error) -> ExtractError,
) -> Result<String, ExtractError> {
    let mut source = String::new();
    reader
        .read_to_string(&mut source)
        .map_err(error_constructor)?;
    Ok(source)
}

type TokenInfo = (Token, Box<str>, logos::Span);
type TokenList = Vec<TokenInfo>;

fn lex_source(source: &str) -> Result<Vec<TokenList>, ExtractError> {
    let mut lexer = Token::lexer(source);

    let mut current = vec![];
    let mut all = vec![];

    let mut depth = 0;
    while let Some(token) = lexer.next() {
        let token_range = Box::from(source.slice(lexer.span()).ok_or(ExtractError::TokenRange)?);
        match token {
            Ok(Token::BraceOpen) => {
                if depth == 0 {
                    all.push(current);
                }
                current = vec![];

                depth += 1;
            }
            Ok(Token::Semicolon) => {
                if depth == 0 {
                    all.push(current);
                }
                current = vec![];
            }
            Ok(Token::BlockComment) => {
                current.push((Token::BlockComment, token_range, lexer.span()));
                if depth == 0 {
                    all.push(current);
                }
                current = vec![];
            }
            Ok(Token::LineComment) => {
                current.push((Token::LineComment, token_range, lexer.span()));
                if depth == 0 {
                    all.push(current);
                }
                current = vec![];
            }
            Ok(Token::BraceClose) => {
                if !current.is_empty() {
                    if depth == 0 {
                        all.push(current);
                    }
                    current = vec![];
                };
                depth -= 1;
            }
            Ok(token) => {
                current.push((token, token_range, lexer.span()));
            }
            // One way to reach this is to have an unterminated string.
            Err(e) => {
                let (line, byte) = line_byte(source, lexer.span());
                return Err(ExtractError::Syntax(line, byte, e));
            }
        }
    }

    if depth != 0 {
        return Err(ExtractError::MissingBodyDelimiter(depth));
    }

    all.push(current);

    Ok(all)
}

fn collect_symbols(
    source: &str,
    all: Vec<Vec<(Token, Box<str>, logos::Span)>>,
    keywords: &HashMap<&str, KeywordType>,
) -> Result<Vec<String>, ExtractError> {
    let mut symbols = vec![];

    for tokens in all {
        let mut token_iter = tokens.iter().peekable();

        skip_whitespace(&mut token_iter);
        if token_iter.peek().is_none() {
            continue;
        }

        match parse_entity(&mut token_iter, keywords) {
            Err(err) => {
                let (src, snippet_start_line, snippet_start_byte) =
                    format_token_list(&tokens, source);
                let (line, byte) = match err.span {
                    Some(sp) => line_byte(source, sp),
                    // At least indicate the approximate location.
                    None => (snippet_start_line, snippet_start_byte),
                };
                return Err(ExtractError::Parse(line, byte, src, err.msg));
            }
            Ok((None, None)) => {}
            Ok((None, Some(short_name))) => symbols.push(short_name),
            Ok((Some(long_name), None)) => symbols.push(long_name),
            Ok((Some(long_name), Some(short_name))) => {
                symbols.push(short_name);
                symbols.push(long_name);
            }
        };
    }

    Ok(symbols)
}

/// A lexer that extracts top-level symbols from a SysML file.
///
/// It is used for handling `index` field of `.meta.json` files.
pub fn top_level_sysml<R: io::Read>(reader: R) -> Result<Vec<String>, ExtractError> {
    let source = read_source(reader, ExtractError::ReadTopLevelSysml)?;

    let all = lex_source(&source)?;

    let keywords = HashMap::from([
        // Simple
        ("abstract", KeywordType::Simple),
        ("public", KeywordType::Simple),
        ("protected", KeywordType::Simple),
        ("private", KeywordType::Simple),
        ("in", KeywordType::Simple),
        ("out", KeywordType::Simple),
        ("inout", KeywordType::Simple),
        ("ref", KeywordType::Simple),
        ("readonly", KeywordType::Simple),
        ("do", KeywordType::Simple),
        ("entry", KeywordType::Simple),
        ("exit", KeywordType::Simple),
        ("enum", KeywordType::Simple),
        ("standard", KeywordType::Simple),
        ("nonunique", KeywordType::Simple),
        ("ordered", KeywordType::Simple),
        ("library", KeywordType::Simple),
        ("package", KeywordType::Simple),
        ("doc", KeywordType::Simple),
        ("attribute", KeywordType::Simple),
        ("alias", KeywordType::Simple),
        ("def", KeywordType::Simple),
        ("metadata", KeywordType::Simple),
        ("item", KeywordType::Simple),
        ("part", KeywordType::Simple),
        ("port", KeywordType::Simple),
        ("calc", KeywordType::Simple),
        ("occurrence", KeywordType::Simple),
        ("event", KeywordType::Simple),
        ("requirement", KeywordType::Simple),
        ("action", KeywordType::Simple),
        ("return", KeywordType::Simple),
        ("enum", KeywordType::Simple),
        ("constraint", KeywordType::Simple),
        ("assert", KeywordType::Simple),
        ("flow", KeywordType::Simple),
        ("subject", KeywordType::Simple),
        ("derived", KeywordType::Simple),
        ("view", KeywordType::Simple),
        ("viewpoint", KeywordType::Simple),
        ("concern", KeywordType::Simple),
        ("verification", KeywordType::Simple),
        ("objective", KeywordType::Simple),
        ("analysis", KeywordType::Simple),
        ("allocation", KeywordType::Simple),
        ("use", KeywordType::Simple),
        ("case", KeywordType::Simple),
        ("connection", KeywordType::Simple),
        ("state", KeywordType::Simple),
        ("rendering", KeywordType::Simple),
        ("interface", KeywordType::Simple),
        ("succession", KeywordType::Simple),
        ("transition", KeywordType::Simple),
        ("message", KeywordType::Simple),
        ("binding", KeywordType::Simple),
        ("result", KeywordType::Simple),
        ("require", KeywordType::Simple),
        ("defined", KeywordType::Simple),
        // Skip a comma separated sequence of names, references, and/or feature chains
        // Sometimes an initial multiplicity is allowed
        (":", KeywordType::MultiReferences),
        ("by", KeywordType::MultiReferences),
        (":>", KeywordType::MultiReferences),
        ("subsets", KeywordType::MultiReferences),
        ("specializes", KeywordType::MultiReferences),
        (":>>", KeywordType::MultiReferences),
        ("redefines", KeywordType::MultiReferences),
        ("end", KeywordType::MultiReferences),
        ("accept", KeywordType::MultiReferences),
        ("via", KeywordType::MultiReferences),
        ("connect", KeywordType::MultiReferencesWithMultiplicity),
        ("to", KeywordType::MultiReferencesWithMultiplicity),
        ("first", KeywordType::MultiReferencesWithMultiplicity),
        ("then", KeywordType::MultiReferencesWithMultiplicity),
        ("satisfy", KeywordType::MultiReferencesWithMultiplicity),
        // Skip the rest of the line
        ("import", KeywordType::SkipRest),
        ("for", KeywordType::SkipRest),
        ("default", KeywordType::SkipRest),
        (":=", KeywordType::SkipRest),
        ("bind", KeywordType::SkipRest),
        ("assign", KeywordType::SkipRest),
        ("then", KeywordType::SkipRest),
    ]);

    collect_symbols(&source, all, &keywords)
}

pub fn top_level_kerml<R: io::Read>(reader: R) -> Result<Vec<String>, ExtractError> {
    let source = read_source(reader, ExtractError::ReadTopLevelKerml)?;

    let all = lex_source(&source)?;

    let keywords = HashMap::from([
        ("public", KeywordType::Simple),
        ("protected", KeywordType::Simple),
        ("private", KeywordType::Simple),
        ("standard", KeywordType::Simple),
        ("library", KeywordType::Simple),
        ("package", KeywordType::Simple),
    ]);

    collect_symbols(&source, all, &keywords)
}

// Returns: (line, byte), both 1-indexed
fn line_byte(source: &str, span: logos::Span) -> (u32, u32) {
    let range = &source.as_bytes()[..span.start];
    let line = range.iter().filter(|x| **x == b'\n').count() as u32 + 1;
    // counts bytes, not chars or graphemes
    let byte = match range.rsplit(|x| *x == b'\n').next() {
        Some(slice) => slice.len() as u32 + 1,
        None => range.len() as u32,
    };
    (line, byte)
}

// Concatenates the tokens and strips start and end whitespace, including newlines.
// Returns (concatenated_string, start_line, start_byte)
fn format_token_list(
    tokens: &[(Token, Box<str>, logos::Span)],
    source: &str,
) -> (String, u32, u32) {
    let mut iter = tokens.iter();
    let Some((_, text, span)) = iter.next() else {
        return (String::new(), 1, 1);
    };
    let mut buf = String::new();
    let trimmed = text.trim_start();
    buf.push_str(trimmed);
    let offset = trimmed.len().saturating_sub(text.len());
    let span_start_with_offset = span.start.saturating_add(offset);
    let (line, byte) = line_byte(source, span_start_with_offset..span.end);
    for (_, text, _) in iter {
        buf.push_str(text);
    }
    buf.truncate(buf.trim_end().len());
    (buf, line, byte)
}
