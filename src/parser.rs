use crate::lexer::Token;

pub fn parse(tokens: &[Token]) -> Result<(), String> {
    let mut parser = Parser { tokens, pos: 0 };
    parser.skip_newlines();
    parser.expect(&Token::App)?;
    parser.ident()?;
    parser.expect(&Token::LBrace)?;
    parser.skip_newlines();
    while !parser.at(&Token::RBrace) {
        parser.expect(&Token::Fn)?;
        parser.ident()?;
        parser.expect(&Token::LParen)?;
        parser.expect(&Token::RParen)?;
        parser.expect(&Token::LBrace)?;
        parser.skip_newlines();
        while !parser.at(&Token::RBrace) {
            parser.statement()?;
            parser.skip_newlines();
        }
        parser.expect(&Token::RBrace)?;
        parser.skip_newlines();
    }
    parser.expect(&Token::RBrace)?;
    parser.skip_newlines();
    parser.expect(&Token::Eof)
}

struct Parser<'a> { tokens: &'a [Token], pos: usize }
impl Parser<'_> {
    fn current(&self) -> &Token { self.tokens.get(self.pos).unwrap_or(&Token::Eof) }
    fn at(&self, token: &Token) -> bool { self.current() == token }
    fn expect(&mut self, token: &Token) -> Result<(), String> {
        if self.at(token) { self.pos += 1; Ok(()) }
        else { Err(format!("expected {token:?}, found {:?}", self.current())) }
    }
    fn ident(&mut self) -> Result<(), String> {
        if matches!(self.current(), Token::Ident(_)) { self.pos += 1; Ok(()) }
        else { Err(format!("expected identifier, found {:?}", self.current())) }
    }
    fn skip_newlines(&mut self) {
        while matches!(self.current(), Token::Newline | Token::Semicolon) { self.pos += 1; }
    }
    fn statement(&mut self) -> Result<(), String> {
        match self.current() {
            Token::Let | Token::Var => {
                self.pos += 1;
                self.ident()?;
                self.expect(&Token::Equal)?;
                self.expression()?;
            }
            Token::Ident(_) => { self.call()?; }
            other => return Err(format!("unsupported statement: {other:?}")),
        }
        if !matches!(self.current(), Token::Newline | Token::Semicolon | Token::RBrace) {
            return Err(format!("expected statement separator, found {:?}", self.current()));
        }
        Ok(())
    }
    fn expression(&mut self) -> Result<(), String> {
        match self.current() {
            Token::String(_) | Token::Number(_) => { self.pos += 1; Ok(()) }
            Token::Ident(_) => {
                self.pos += 1;
                if self.at(&Token::LParen) {
                    self.pos += 1;
                    self.arguments()?;
                    self.expect(&Token::RParen)?;
                }
                Ok(())
            }
            other => Err(format!("expected expression, found {other:?}")),
        }
    }
    fn call(&mut self) -> Result<(), String> {
        self.ident()?;
        self.expect(&Token::LParen)?;
        self.arguments()?;
        self.expect(&Token::RParen)
    }
    fn arguments(&mut self) -> Result<(), String> {
        if self.at(&Token::RParen) { return Ok(()); }
        self.expression()?;
        while self.at(&Token::Comma) {
            self.pos += 1;
            self.expression()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;
    #[test]
    fn parses_hello_world() {
        let source = "app Hello {\n fn main() {\n let message = \"Hello, World!\"\n print(message)\n }\n}";
        assert!(parse(&lex(source).unwrap()).is_ok());
    }
    #[test]
    fn rejects_missing_brace() {
        assert!(parse(&lex("app Hello { fn main() { print(\"Hi\") }").unwrap()).is_err());
    }
}
