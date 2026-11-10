#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    App, Fn, Let, Var, Loop, When, Else, While, Until, Return,
    Ident(String), Number(String), String(String),
    LBrace, RBrace, LParen, RParen, Comma, Equal, Semicolon,
    Plus, Minus, Star, Slash, Less, Greater, EqualEqual, BangEqual, LessEqual, GreaterEqual,
    Newline, Eof,
}

pub fn lex(source: &str) -> Result<Vec<Token>, String> {
    let mut chars = source.trim_start_matches('\u{feff}').chars().peekable();
    let mut tokens = Vec::new();
    while let Some(ch) = chars.next() {
        match ch {
            ' ' | '\t' | '\r' => {}
            '\n' => tokens.push(Token::Newline),
            '{' => tokens.push(Token::LBrace),
            '}' => tokens.push(Token::RBrace),
            '(' => tokens.push(Token::LParen),
            ')' => tokens.push(Token::RParen),
            ',' => tokens.push(Token::Comma),
            '=' => tokens.push(if chars.peek() == Some(&'=') { chars.next(); Token::EqualEqual } else { Token::Equal }),
            '!' if chars.peek() == Some(&'=') => { chars.next(); tokens.push(Token::BangEqual); }
            '<' => tokens.push(if chars.peek() == Some(&'=') { chars.next(); Token::LessEqual } else { Token::Less }),
            '>' => tokens.push(if chars.peek() == Some(&'=') { chars.next(); Token::GreaterEqual } else { Token::Greater }),
            '+' => tokens.push(Token::Plus),
            '-' => tokens.push(Token::Minus),
            '*' => tokens.push(Token::Star),
            ';' => tokens.push(Token::Semicolon),
            '/' if chars.peek() == Some(&'/') => {
                for c in chars.by_ref() {
                    if c == '\n' { tokens.push(Token::Newline); break; }
                }
            }
            '/' => tokens.push(Token::Slash),
            '"' => {
                let mut value = String::new();
                let mut closed = false;
                while let Some(c) = chars.next() {
                    if c == '"' { closed = true; break; }
                    if c == '\\' {
                        match chars.next() {
                            Some('n') => value.push('\n'),
                            Some('t') => value.push('\t'),
                            Some('"') => value.push('"'),
                            Some('\\') => value.push('\\'),
                            Some(other) => return Err(format!("unsupported escape: \\{other}")),
                            None => return Err("unterminated string escape".into()),
                        }
                    } else { value.push(c); }
                }
                if !closed { return Err("unterminated string literal".into()); }
                tokens.push(Token::String(value));
            }
            c if c.is_ascii_digit() => {
                let mut s = c.to_string();
                while let Some(c) = chars.peek() {
                    if c.is_ascii_digit() { s.push(*c); chars.next(); } else { break; }
                }
                tokens.push(Token::Number(s));
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let mut s = c.to_string();
                while let Some(c) = chars.peek() {
                    if c.is_ascii_alphanumeric() || *c == '_' { s.push(*c); chars.next(); } else { break; }
                }
                tokens.push(match s.as_str() {
                    "app" => Token::App, "fn" => Token::Fn,
                    "let" => Token::Let, "var" => Token::Var,
                    "loop" => Token::Loop, "when" => Token::When,
                    "else" => Token::Else, "while" => Token::While,
                    "until" => Token::Until, "return" => Token::Return,
                    _ => Token::Ident(s),
                });
            }
            other => return Err(format!("unexpected character: {other}")),
        }
    }
    tokens.push(Token::Eof);
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tokenizes_hello_world() {
        let tokens = lex("app Hello { fn main() { print(\"Hi\") } }").unwrap();
        assert_eq!(tokens[0], Token::App);
        assert!(tokens.contains(&Token::String("Hi".into())));
    }
    #[test]
    fn accepts_utf8_bom() {
        let tokens = lex("\u{feff}app Hello { fn main() { print(\"Hi\") } }").unwrap();
        assert_eq!(tokens[0], Token::App);
    }
    #[test]
    fn rejects_unclosed_string() {
        assert!(lex("print(\"Hi)").is_err());
    }
}
