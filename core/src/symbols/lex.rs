// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use logos::{Lexer, Logos};
use thiserror::Error;

#[derive(Default, Debug, Clone, PartialEq, Error)]
pub enum LexingError {
    #[error("unterminated `{0}`")]
    Unterminated(&'static str),
    #[error("{0}")]
    Unexpected(&'static str),
    #[default]
    #[error("unknown parsing error")]
    Unknown,
}

// Block comments, strings, and quoted names are easier to handle manually, as
// Logos regexes are too greedy.
fn lex_block_comment(lex: &mut Lexer<'_, Token>) -> Result<(), LexingError> {
    let rem = lex.remainder();
    let mut close_pos = rem.find("*/").ok_or(LexingError::Unterminated("/*"))?;

    while rem[..close_pos + 2].ends_with("\\*/") {
        close_pos += rem[close_pos + 2..]
            .find("*/")
            .ok_or(LexingError::Unterminated("/*"))?
            + 2;
    }

    lex.bump(close_pos + 2);

    Ok(())
}

fn lex_string(lex: &mut Lexer<'_, Token>) -> Result<(), LexingError> {
    let rem = lex.remainder();
    let mut close_pos = rem.find("\"").ok_or(LexingError::Unterminated("\""))?;

    while rem[..close_pos + 1].ends_with("\\\"") {
        close_pos += rem[close_pos + 1..]
            .find("\"")
            .ok_or(LexingError::Unterminated("\""))?
            + 1;
    }

    lex.bump(close_pos + 1);

    Ok(())
}

fn lex_quoted(lex: &mut Lexer<'_, Token>) -> Result<(), LexingError> {
    let rem = lex.remainder();
    let mut close_pos = rem.find("'").ok_or(LexingError::Unterminated("'"))?;

    while rem[..close_pos + 1].ends_with("\\'") {
        close_pos += rem[close_pos + 1..]
            .find("'")
            .ok_or(LexingError::Unterminated("'"))?
            + 1;
    }

    lex.bump(close_pos + 1);

    Ok(())
}

fn lex_symbol(lex: &mut Lexer<'_, Token>) -> Result<Token, LexingError> {
    let rem = lex.remainder();

    let this_char: &str = lex.slice();

    let mut symbol_block_close_pos = rem
        .find(|c: char| {
            c.is_alphanumeric()
                || c.is_whitespace()
                || c == '\''
                || c == '"'
                || c == '{'
                || c == '}'
                || c == ';'
                || c == ','
                || c == '['
                || c == ']'
        })
        .ok_or(LexingError::Unexpected(
            // TODO: improve wording
            "expected a block of symbols, found none",
        ))?;

    if let Some(line_comment_start_pos) = rem[..symbol_block_close_pos].find("//") {
        symbol_block_close_pos = line_comment_start_pos;
    }

    if let Some(block_comment_start_pos) = rem[..symbol_block_close_pos].find("/*") {
        symbol_block_close_pos = block_comment_start_pos;
    }

    if this_char.starts_with('<') {
        return Ok(Token::LT);
    } else if this_char.starts_with('>') {
        return Ok(Token::GT);
    } else if this_char.starts_with(':') && rem.starts_with(":") {
        lex.bump(1);
        return Ok(Token::DoubleColon);
    } else if this_char.starts_with('=') {
        return Ok(Token::Equals);
    } else if this_char.starts_with('.') {
        return Ok(Token::Period);
    }

    lex.bump(symbol_block_close_pos);

    Ok(Token::OtherSymbol)

    // let mut close_pos = rem.find("'").ok_or(LexingError::Unterminated("'"))?;

    // while rem[..close_pos].ends_with("\\'") {
    //     close_pos = rem.find("'").ok_or(LexingError::Unterminated("'"))?;
    // }

    // lex.bump(close_pos + 1);

    // Ok(lex.slice())
}

#[derive(Logos, Debug, PartialEq)]
#[logos(error = LexingError)]
pub enum Token {
    #[regex(r"\s+")]
    Space,

    #[token("#")]
    UserKeywordStart,

    #[token("{")]
    BraceOpen,

    #[token("}")]
    BraceClose,

    #[token(";")]
    Semicolon,

    #[token(",")]
    Comma,

    // Thanks https://github.com/maciejhirsz/logos/issues/133
    #[regex(r"(\p{XID_Start}|_)\p{XID_Continue}*")]
    #[regex(r"[0-9]+")]
    OtherIdentifier,

    // TODO: Figure out properly where comments terminate
    // an element declaration. E.g.
    // `doc /* look no semicolon... */`
    #[token("/*", |lex| lex_block_comment(lex))]
    #[token("//*", |lex| lex_block_comment(lex))]
    BlockComment,

    #[regex(r"//[^\*][^\n]*", priority = 10)]
    LineComment,

    #[token("\"", |lex| lex_string(lex))]
    String,

    #[token("'", |lex| lex_quoted(lex))]
    Quoted,

    #[token("(")]
    OpenParen,

    #[token(")")]
    CloseParen,

    #[token("[")]
    OpenSquare,

    #[token("]")]
    CloseSquare,

    //#[token("::")]
    DoubleColon,
    //#[token("<")]
    LT,
    //#[token(">")]
    GT,
    //#[token("=")]
    Equals,
    //#[token(".")]
    Period,

    #[regex(r".", |lex| lex_symbol(lex), priority=0)]
    OtherSymbol,
}
