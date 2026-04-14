// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use super::*;

fn tokens_match(src: &str, tokens: &[Token]) -> Result<(), Box<dyn std::error::Error>> {
    let tokenized: Result<Vec<Token>, _> = Token::lexer(src).collect();
    let tokenized = tokenized.unwrap_or_else(|e| panic!("failed to tokenize {src:?}: {e}"));
    assert_eq!(tokenized.as_slice(), tokens);
    Ok(())
}

fn unclosed_comment_doesnt_parse(src: &str) {
    let tokens: Result<Vec<Token>, _> = Token::lexer(src).collect();
    assert!(
        matches!(tokens, Err(LexingError::Unterminated("/*"))),
        "src: {src:?}"
    );
}

#[test]
fn lex_line_comment() -> Result<(), Box<dyn std::error::Error>> {
    let src1 = "//";
    let src2 = "//\n";
    let src3 = "//\r\n";
    let src4 = "//\n\n";
    let src5 = "//\r\n\r\n";
    let src6 = "//abc";
    let src7 = "//abc\n";
    let src8 = "//abc\r\n";
    let src9 = "//abc\n\n";
    let src10 = "//abc\r\n\r\n";
    let src22 = "//\nabc";
    let src23 = "//\r\nabc";
    let src24 = "//\n\nabc";
    let src25 = "//\r\n\r\nabc";
    let src27 = "//abc\nabc";
    let src28 = "//abc\r\nabc";
    let src29 = "//abc\n\nabc";
    let src30 = "//abc\r\n\r\nabc";
    let src31 = "// *";
    let src32 = "// *\n";
    let src33 = "// *\r\n";
    let src34 = "// *\n\n";
    let src35 = "// *\r\n\r\n";
    // let src36 = "//\n*"; // this doesn't parse
    // let src37 = "//\r\n*";
    let src38 = "//\n*\n";
    let src39 = "//\r\n*\r\n";
    for s in [src1, src2, src3, src6, src7, src8, src31, src32, src33] {
        tokens_match(s, &[Token::LineComment])?;
    }
    for s in [src4, src5, src9, src10, src34, src35] {
        tokens_match(s, &[Token::LineComment, Token::Space])?;
    }
    for s in [src22, src23, src27, src28] {
        tokens_match(s, &[Token::LineComment, Token::OtherIdentifier])?;
    }
    for s in [src38, src39] {
        tokens_match(s, &[Token::LineComment, Token::OtherSymbol, Token::Space])?;
    }
    for s in [src24, src25, src29, src30] {
        tokens_match(
            s,
            &[Token::LineComment, Token::Space, Token::OtherIdentifier],
        )?;
    }

    // None of these must match line comment
    let src11 = "//*";
    let src12 = "//*\n";
    let src13 = "//*\r\n";
    let src14 = "//*\n\n";
    let src15 = "//*\r\n\r\n";
    let src16 = "//*abc";
    let src17 = "//*abc\n";
    let src18 = "//*abc\r\n";
    let src19 = "//*abc\n\n";
    let src20 = "//*abc\r\n\r\n";
    for s in [src11, src12, src13, src16, src17, src18] {
        unclosed_comment_doesnt_parse(s);
    }
    for s in [src14, src15, src19, src20] {
        unclosed_comment_doesnt_parse(s);
    }
    Ok(())
}
