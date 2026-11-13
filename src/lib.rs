pub mod crud;
pub mod native_crud;
pub mod database;
pub mod postgres_repository;
pub mod crud_schema;
pub mod lexer;
pub mod parser;
pub mod runtime;

pub fn check(source: &str) -> Result<(), String> {
    let tokens = lexer::lex(source)?;
    parser::parse(&tokens)?;
    Ok(())
}

pub fn run(source: &str) -> Result<String, String> {
    let tokens = lexer::lex(source)?;
    let program = parser::parse(&tokens)?;
    runtime::run(&program)
}

pub mod project;

pub mod identity;

pub mod user_auth;

pub mod tenant;
