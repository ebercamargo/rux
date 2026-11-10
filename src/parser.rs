use crate::lexer::Token;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub name: String,
    pub functions: Vec<Function>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Function {
    pub name: String,
    pub body: Vec<Statement>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Statement {
    Bind { name: String, mutable: bool, value: Expr },
    Call { name: String, args: Vec<Expr> },
    Assign { name: String, value: Expr },
    When { condition: Expr, then_body: Vec<Statement>, else_body: Vec<Statement> },
    Loop { count: Expr, body: Vec<Statement> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    Text(String),
    Number(String),
    Variable(String),
    Bool(bool),
}

pub fn parse(tokens: &[Token]) -> Result<Program, String> {
    let mut parser = Parser { tokens, pos: 0 };
    parser.skip_separators();
    parser.expect(&Token::App)?;
    let name = parser.ident()?;
    parser.expect(&Token::LBrace)?;
    parser.skip_separators();
    let mut functions = Vec::new();
    while !parser.at(&Token::RBrace) {
        if parser.at(&Token::Eof) {
            return Err("unexpected end of file in app".into());
        }
        parser.expect(&Token::Fn)?;
        let function_name = parser.ident()?;
        parser.expect(&Token::LParen)?;
        parser.expect(&Token::RParen)?;
        parser.expect(&Token::LBrace)?;
        parser.skip_separators();
        let mut body = Vec::new();
        while !parser.at(&Token::RBrace) {
            if parser.at(&Token::Eof) {
                return Err("unexpected end of file in function".into());
            }
            body.push(parser.statement()?);
            parser.skip_separators();
        }
        parser.expect(&Token::RBrace)?;
        functions.push(Function { name: function_name, body });
        parser.skip_separators();
    }
    parser.expect(&Token::RBrace)?;
    parser.skip_separators();
    parser.expect(&Token::Eof)?;
    Ok(Program { name, functions })
}

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
}

impl Parser<'_> {
    fn current(&self) -> &Token {
        self.tokens.get(self.pos).unwrap_or(&Token::Eof)
    }

    fn at(&self, token: &Token) -> bool {
        self.current() == token
    }

    fn expect(&mut self, token: &Token) -> Result<(), String> {
        if self.at(token) {
            self.pos += 1;
            Ok(())
        } else {
            Err(format!("expected {token:?}, found {:?}", self.current()))
        }
    }

    fn ident(&mut self) -> Result<String, String> {
        match self.current() {
            Token::Ident(value) => {
                let result = value.clone();
                self.pos += 1;
                Ok(result)
            }
            other => Err(format!("expected identifier, found {other:?}")),
        }
    }

    fn skip_separators(&mut self) {
        while matches!(self.current(), Token::Newline | Token::Semicolon) {
            self.pos += 1;
        }
    }

    fn block(&mut self) -> Result<Vec<Statement>, String> {
        self.expect(&Token::LBrace)?;
        self.skip_separators();
        let mut body = Vec::new();
        while !self.at(&Token::RBrace) {
            if self.at(&Token::Eof) { return Err("unexpected end of file in block".into()); }
            body.push(self.statement()?);
            self.skip_separators();
        }
        self.expect(&Token::RBrace)?;
        Ok(body)
    }

    fn statement(&mut self) -> Result<Statement, String> {
        let statement = match self.current() {
            Token::Let | Token::Var => {
                let mutable = self.at(&Token::Var);
                self.pos += 1;
                let name = self.ident()?;
                self.expect(&Token::Equal)?;
                let value = self.expression()?;
                Statement::Bind { name, mutable, value }
            }
            Token::When => {
                self.pos += 1;
                let condition = self.expression()?;
                let then_body = self.block()?;
                self.skip_separators();
                let else_body = if self.at(&Token::Else) {
                    self.pos += 1;
                    self.block()?
                } else { Vec::new() };
                return Ok(Statement::When { condition, then_body, else_body });
            }
            Token::Loop => {
                self.pos += 1;
                let count = self.expression()?;
                let body = self.block()?;
                return Ok(Statement::Loop { count, body });
            }
            Token::Ident(_) => {
                let name = self.ident()?;
                if self.at(&Token::Equal) {
                    self.pos += 1;
                    let value = self.expression()?;
                    return Ok(Statement::Assign { name, value });
                }
                self.expect(&Token::LParen)?;
                let mut args = Vec::new();
                if !self.at(&Token::RParen) {
                    args.push(self.expression()?);
                    while self.at(&Token::Comma) {
                        self.pos += 1;
                        args.push(self.expression()?);
                    }
                }
                self.expect(&Token::RParen)?;
                Statement::Call { name, args }
            }
            other => return Err(format!("unsupported statement: {other:?}")),
        };
        if !matches!(
            self.current(),
            Token::Newline | Token::Semicolon | Token::RBrace
        ) {
            return Err(format!(
                "expected statement separator, found {:?}",
                self.current()
            ));
        }
        Ok(statement)
    }

    fn expression(&mut self) -> Result<Expr, String> {
        let value = match self.current() {
            Token::String(text) => Expr::Text(text.clone()),
            Token::Number(number) => Expr::Number(number.clone()),
            Token::Ident(name) if name == "true" => Expr::Bool(true),
            Token::Ident(name) if name == "false" => Expr::Bool(false),
            Token::Ident(name) => Expr::Variable(name.clone()),
            other => return Err(format!("expected expression, found {other:?}")),
        };
        self.pos += 1;
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;

    #[test]
    fn parses_hello_world() {
        let source = "app Hello {\n fn main() {\n let message = \"Hello, World!\"\n print(message)\n }\n}";
        let program = parse(&lex(source).unwrap()).unwrap();
        assert_eq!(program.name, "Hello");
        assert_eq!(program.functions[0].body.len(), 2);
    }

    #[test]
    fn rejects_missing_brace() {
        assert!(parse(&lex("app Hello { fn main() { print(\"Hi\") }").unwrap()).is_err());
    }

    #[test]
    fn rejects_unsupported_nested_call() {
        assert!(parse(&lex("app A { fn main() { print(other()) } }").unwrap()).is_err());
    }
}
