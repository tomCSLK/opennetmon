//! Tokenizer for SMI (MIB) text: identifiers, numbers, strings, punctuation; comments are dropped.
//! Tolerant: unknown characters become `Token::Other` instead of an error.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    /// Identifier or keyword (letters, digits, hyphens; e.g. `OBJECT-TYPE`, `ifIndex`).
    Ident(String),
    /// Decimal number (may be negative, as in ranges).
    Number(String),
    /// Quoted string with the quotes removed (may span lines).
    Str(String),
    /// Binary or hexadecimal string such as `'00'H`.
    BinStr(String),
    Assign,
    LBrace,
    RBrace,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Range,
    Comma,
    Semicolon,
    Pipe,
    Dot,
    Other(char),
}

/// A token with its 1-based line number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spanned {
    pub token: Token,
    pub line: usize,
}

pub fn tokenize(text: &str) -> Vec<Spanned> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let mut line = 1;
    while i < chars.len() {
        let c = chars[i];
        if c == '\n' {
            line += 1;
            i += 1;
            continue;
        }
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        // Comment: "--" up to the end of the line or the next "--".
        if c == '-' && chars.get(i + 1) == Some(&'-') {
            i += 2;
            while i < chars.len() && chars[i] != '\n' {
                if chars[i] == '-' && chars.get(i + 1) == Some(&'-') {
                    i += 2;
                    break;
                }
                i += 1;
            }
            continue;
        }
        let start_line = line;
        let token = if c == '"' {
            let mut s = String::new();
            i += 1;
            while i < chars.len() {
                if chars[i] == '"' {
                    if chars.get(i + 1) == Some(&'"') {
                        s.push('"');
                        i += 2;
                        continue;
                    }
                    break;
                }
                if chars[i] == '\n' {
                    line += 1;
                }
                if chars[i] != '\r' {
                    s.push(chars[i]);
                }
                i += 1;
            }
            i += 1; // closing quote (or end of input)
            Token::Str(s)
        } else if c == '\'' {
            let mut s = String::new();
            i += 1;
            while i < chars.len() && chars[i] != '\'' {
                if chars[i] == '\n' {
                    line += 1;
                }
                s.push(chars[i]);
                i += 1;
            }
            i += 1;
            // Suffix H or B.
            if i < chars.len() && matches!(chars[i], 'H' | 'h' | 'B' | 'b') {
                i += 1;
            }
            Token::BinStr(s)
        } else if c == ':' && chars.get(i + 1) == Some(&':') && chars.get(i + 2) == Some(&'=') {
            i += 3;
            Token::Assign
        } else if c == '.' && chars.get(i + 1) == Some(&'.') {
            i += 2;
            Token::Range
        } else if c.is_ascii_digit()
            || (c == '-' && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit()))
        {
            let mut s = String::from(c);
            i += 1;
            while i < chars.len() && chars[i].is_ascii_digit() {
                s.push(chars[i]);
                i += 1;
            }
            Token::Number(s)
        } else if c.is_ascii_alphabetic() {
            let mut s = String::from(c);
            i += 1;
            while i < chars.len() {
                let d = chars[i];
                // A hyphen belongs to the identifier unless it starts a comment.
                if d.is_ascii_alphanumeric()
                    || d == '_'
                    || (d == '-' && chars.get(i + 1) != Some(&'-'))
                {
                    s.push(d);
                    i += 1;
                } else {
                    break;
                }
            }
            Token::Ident(s)
        } else {
            i += 1;
            match c {
                '{' => Token::LBrace,
                '}' => Token::RBrace,
                '(' => Token::LParen,
                ')' => Token::RParen,
                '[' => Token::LBracket,
                ']' => Token::RBracket,
                ',' => Token::Comma,
                ';' => Token::Semicolon,
                '|' => Token::Pipe,
                '.' => Token::Dot,
                other => Token::Other(other),
            }
        };
        out.push(Spanned {
            token,
            line: start_line,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(text: &str) -> Vec<Token> {
        tokenize(text).into_iter().map(|s| s.token).collect()
    }

    fn ident(s: &str) -> Token {
        Token::Ident(s.to_owned())
    }

    #[test]
    fn identifiers_with_hyphens_numbers_and_punctuation() {
        assert_eq!(
            tokens("ifIndex OBJECT-TYPE ::= { ifEntry 1 }"),
            [
                ident("ifIndex"),
                ident("OBJECT-TYPE"),
                Token::Assign,
                Token::LBrace,
                ident("ifEntry"),
                Token::Number("1".to_owned()),
                Token::RBrace
            ]
        );
        assert_eq!(
            tokens("INTEGER (-2147483648..2147483647)"),
            [
                ident("INTEGER"),
                Token::LParen,
                Token::Number("-2147483648".to_owned()),
                Token::Range,
                Token::Number("2147483647".to_owned()),
                Token::RParen
            ]
        );
    }

    #[test]
    fn comments_end_at_line_end_or_at_the_next_double_hyphen() {
        assert_eq!(tokens("a -- comment\nb"), [ident("a"), ident("b")]);
        assert_eq!(tokens("a -- inline -- b"), [ident("a"), ident("b")]);
        assert_eq!(
            tokens("a--x"),
            [ident("a")],
            "a hyphen pair ends the identifier"
        );
        assert_eq!(tokens("-- only a comment"), []);
    }

    #[test]
    fn strings_span_lines_and_keep_line_numbers_right() {
        let spanned =
            tokenize("DESCRIPTION\r\n  \"first\r\n  second \"\"quoted\"\"\"\r\nSTATUS current");
        assert_eq!(
            spanned[1].token,
            Token::Str("first\n  second \"quoted\"".to_owned())
        );
        assert_eq!(
            spanned[2],
            Spanned {
                token: ident("STATUS"),
                line: 4
            }
        );
        assert_eq!(spanned[2].line, 4);
    }

    #[test]
    fn binary_strings_brackets_and_unknown_characters() {
        assert_eq!(
            tokens("DEFVAL { '00'H } [APPLICATION 1] @"),
            [
                ident("DEFVAL"),
                Token::LBrace,
                Token::BinStr("00".to_owned()),
                Token::RBrace,
                Token::LBracket,
                ident("APPLICATION"),
                Token::Number("1".to_owned()),
                Token::RBracket,
                Token::Other('@')
            ]
        );
    }

    #[test]
    fn unterminated_strings_do_not_panic() {
        assert_eq!(tokens("\"open"), [Token::Str("open".to_owned())]);
        assert_eq!(tokens("'0F"), [Token::BinStr("0F".to_owned())]);
    }
}
