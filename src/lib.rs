pub mod crud;
pub mod native_crud;
pub mod database;
pub mod crud_schema;
pub mod lexer;
pub mod parser;
pub mod runtime;

pub fn check(source: &str) -> Result<(), String> {
    let tokens = lexer::lex(source)?;
    let program = parser::parse(&tokens)?;
    validate_program(&program)
}

pub fn run(source: &str) -> Result<String, String> {
    let tokens = lexer::lex(source)?;
    let program = parser::parse(&tokens)?;
    validate_program(&program)?;
    runtime::run(&program)
}

fn validate_program(program: &parser::Program) -> Result<(), String> {
    use std::collections::HashSet;
    let mut names = HashSet::new();
    for function in &program.functions {
        if !names.insert(function.name.as_str()) {
            return Err(format!("duplicate function: {}", function.name));
        }
        let mut params = HashSet::new();
        for param in &function.params {
            if !params.insert(param.as_str()) {
                return Err(format!("duplicate parameter: {param} in function {}", function.name));
            }
        }
    }
    match program.functions.iter().find(|f| f.name == "main") {
        None => Err("missing entry point: fn main()".into()),
        Some(main) if !main.params.is_empty() => Err("entry point fn main() must not take arguments".into()),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod semantic_tests {
    use super::*;
    #[test]
    fn rejects_duplicate_functions() {
        assert!(check("app A { fn main() {} fn main() {} }").unwrap_err().contains("duplicate function"));
    }
    #[test]
    fn rejects_duplicate_parameters() {
        assert!(check("app A { fn f(x, x) {} fn main() {} }").unwrap_err().contains("duplicate parameter"));
    }
    #[test]
    fn requires_zero_argument_main() {
        assert!(check("app A { fn main(x) {} }").unwrap_err().contains("must not take arguments"));
        assert!(check("app A { fn helper() {} }").unwrap_err().contains("missing entry point"));
    }
}
