// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Module for extracting symbols from KerML or SysML files.
//!
//! The main function is `top_level`, which returns top level symbols for a
//! given file. We need to know the top level symbols to correctly populate
//! field `index` of `.meta.json` files.

mod lex;

use std::{collections::HashMap, iter::Peekable};

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

fn parse_name<'a, I: Iterator<Item = &'a (Token, Box<str>)>>(
    token_iter: &mut Peekable<I>,
) -> Result<String, String> {
    match token_iter.next() {
        None => Err("Empty name".to_string()),
        Some((Token::Quoted, original)) => Ok(original.trim_matches('\'').to_string()),
        Some((Token::OtherIdentifier, original)) => Ok(original.to_string()),
        Some((invalid_token, original)) => Err(format!(
            "Invalid token of type {:?}, expected a name component: {}",
            invalid_token, original
        )),
    }
}

fn parse_reference_or_chain<'a, I: Iterator<Item = &'a (Token, Box<str>)>>(
    token_iter: &mut Peekable<I>,
) -> Result<Vec<String>, String> {
    let mut result = vec![];

    loop {
        let name_component = parse_name(token_iter)?;

        result.push(name_component);

        match token_iter.peek() {
            Some((Token::DoubleColon, _)) | Some((Token::Period, _)) => {
                token_iter.next();
            }
            _ => {
                break;
            }
        }
    }

    Ok(result)
}

fn skip_nested<'a, I: Iterator<Item = &'a (Token, Box<str>)>>(
    token_iter: &mut Peekable<I>,
    open: Token,
    close: Token,
) -> Result<(), String> {
    let mut nesting = 0;

    loop {
        match token_iter.next() {
            Some((token, _)) => {
                if token == &open {
                    nesting += 1;
                } else if token == &close {
                    nesting -= 1;
                    if nesting == 0 {
                        break;
                    }
                }
            }
            None => return Err(format!("Unmatched {:?}", open)),
        }
    }

    Ok(())
}

fn skip_whitespace<'a, I: Iterator<Item = &'a (Token, Box<str>)>>(token_iter: &mut Peekable<I>) {
    while let Some((Token::Space, _)) = token_iter.peek() {
        token_iter.next();
    }
}

fn maybe_skip_muliplicity<'a, I: Iterator<Item = &'a (Token, Box<str>)>>(
    token_iter: &mut Peekable<I>,
) -> Result<(), String> {
    if let Some((Token::OpenSquare, _)) = token_iter.peek() {
        skip_nested(token_iter, Token::OpenSquare, Token::CloseSquare)?;
    };

    Ok(())
}

fn skip_list_of_refs<'a, I: Iterator<Item = &'a (Token, Box<str>)>>(
    token_iter: &mut Peekable<I>,
) -> Result<(), String> {
    loop {
        let _ = parse_reference_or_chain(token_iter)?;

        skip_whitespace(token_iter);

        match token_iter.peek() {
            Some((Token::Comma, _)) => {
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

fn parse_entity<'a, I: Iterator<Item = &'a (Token, Box<str>)>>(
    token_iter: &'a mut Peekable<I>,
) -> Result<(Option<String>, Option<String>), String> {
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

    let mut long_name = None;
    let mut short_name = None;

    while let Some((token, original)) = token_iter.peek() {
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
            Token::Comma => return Err("floating comma".to_string()),
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
                                return Err(format!("Warn: Got a floating reference: {:?}", names));
                            }
                        };

                        match long_name {
                            None => {
                                long_name = Some(this_name.clone());
                            }
                            Some(_) => {
                                return Err(format!("Unknown name '{}'", this_name));
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
            Token::CloseParen => panic!("Floating closing paren"),
            Token::OpenSquare => skip_nested(token_iter, Token::OpenSquare, Token::CloseSquare)?,
            Token::CloseSquare => panic!("Floating closing square bracket"),
            Token::DoubleColon => panic!("Floating double colon"),
            Token::LT => {
                token_iter.next();
                let this_name = parse_name(token_iter)?;
                match short_name {
                    None => {
                        short_name = Some(this_name);
                    }
                    Some(_) => {
                        return Err(format!("Unknown name {}", this_name));
                    }
                };
                match token_iter.next() {
                    None => return Err("Unclosed short-name <".to_string()),
                    Some((Token::GT, _)) => {}
                    Some((invalid_token, original)) => {
                        return Err(format!(
                            "Expected < found {:?}: {}",
                            invalid_token, original
                        ));
                    }
                };
            }
            Token::GT => return Err("Floating <".to_string()),
            Token::Equals => {
                break;
            }
            Token::Period => return Err("Floating period".to_string()),
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
                    return Err(format!("Floating unknown symbol '{}'", original));
                }
            },
        };
    }

    Ok((long_name, short_name))
}

#[derive(Debug, Error)]
pub enum ExtractError {
    #[error("failed to read file to extract symbols: {0}")]
    ReadTopLevelSysml(std::io::Error),
    #[error("syntax error at line {0}, byte {1}:\n{2}")]
    SyntaxError(u32, u32, LexingError),
    #[error(
        "missing body delimiter: brace '{{}}' nesting depth is {0} (should be 0) at the end of file"
    )]
    MissingBodyDelimiter(i32),
    #[error("unable to get token range")]
    TokenRangeError,
    #[error("failed to parse\n{0}")]
    ParseError(String),
}

/// A lexer that extracts top-level symbols from a SysML file.
///
/// It is used for handling `index` field of `.meta.json` files.
pub fn top_level_sysml<R: std::io::Read>(mut reader: R) -> Result<Vec<String>, ExtractError> {
    let source = {
        let mut source = String::new();
        reader
            .read_to_string(&mut source)
            .map_err(ExtractError::ReadTopLevelSysml)?;
        source
    };

    let mut lexer = Token::lexer(&source);

    let mut current = vec![];
    let mut all = vec![];

    let mut depth = 0;
    while let Some(token) = lexer.next() {
        let token_range = Box::from(
            source
                .slice(lexer.span())
                .ok_or(ExtractError::TokenRangeError)?,
        );
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
                current.push((Token::BlockComment, token_range));
                if depth == 0 {
                    all.push(current);
                }
                current = vec![];
            }
            Ok(Token::LineComment) => {
                current.push((Token::LineComment, token_range));
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
                current.push((token, token_range));
            }
            // One way to reach this is to have an unterminated string.
            Err(e) => {
                let range = source[..lexer.span().start].as_bytes();
                let line = range.iter().filter(|x| **x == b'\n').count() as u32 + 1;
                // counts bytes, not chars or graphemes
                let byte = match range.rsplit(|x| *x == b'\n').next() {
                    Some(slice) => slice.len() as u32 + 1,
                    None => range.len() as u32,
                };
                return Err(ExtractError::SyntaxError(line, byte, e));
            }
        }
    }

    if depth != 0 {
        return Err(ExtractError::MissingBodyDelimiter(depth));
    }

    all.push(current);

    let mut symbols = vec![];

    for tokens in all {
        let mut token_iter = tokens.iter().peekable();

        skip_whitespace(&mut token_iter);
        if token_iter.peek().is_none() {
            continue;
        }

        match parse_entity(&mut token_iter) {
            Err(err) => {
                return Err(ExtractError::ParseError(format!(
                    "'{}': {}",
                    format_token_list(&tokens[..]),
                    err
                )));
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

fn format_token_list(list: &[(Token, Box<str>)]) -> String {
    let mut buf = String::new();
    for (_, text) in list {
        buf.push_str(text);
    }
    buf
}
