pub mod lexer;
pub mod parser;

pub fn check(source: &str) -> Result<(), String> {
    let tokens = lexer::lex(source)?;
    parser::parse(&tokens)
}
