use std::collections::HashMap;

use crate::parser::{Expr, Program, Statement};

pub fn run(program: &Program) -> Result<String, String> {
    let main = program
        .functions
        .iter()
        .find(|function| function.name == "main")
        .ok_or("missing fn main()")?;

    let mut variables: HashMap<String, String> = HashMap::new();
    let mut output = String::new();

    for statement in &main.body {
        match statement {
            Statement::Bind { name, value, .. } => {
                if variables.contains_key(name) {
                    return Err(format!("duplicate variable: {name}"));
                }
                let evaluated = evaluate(value, &variables)?;
                variables.insert(name.clone(), evaluated);
            }
            Statement::Call { name, args } if name == "print" => {
                let values: Result<Vec<_>, _> =
                    args.iter().map(|arg| evaluate(arg, &variables)).collect();
                output.push_str(&values?.join(" "));
                output.push('\n');
            }
            Statement::Call { name, .. } => {
                return Err(format!("unknown function: {name}"));
            }
        }
    }
    Ok(output)
}

fn evaluate(expr: &Expr, variables: &HashMap<String, String>) -> Result<String, String> {
    match expr {
        Expr::Text(value) | Expr::Number(value) => Ok(value.clone()),
        Expr::Variable(name) => variables
            .get(name)
            .cloned()
            .ok_or_else(|| format!("undefined variable: {name}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{lexer, parser};

    fn execute(source: &str) -> Result<String, String> {
        run(&parser::parse(&lexer::lex(source)?)?)
    }

    #[test]
    fn hello_world_runs() {
        let source = "app Hello { fn main() { let message = \"Hello, World!\"; print(message) } }";
        assert_eq!(execute(source).unwrap(), "Hello, World!\n");
    }

    #[test]
    fn rejects_undefined_variable() {
        assert!(execute("app A { fn main() { print(missing) } }").is_err());
    }

    #[test]
    fn rejects_unknown_function() {
        assert!(execute("app A { fn main() { unknown() } }").is_err());
    }

    #[test]
    fn requires_main() {
        assert!(execute("app A { fn other() {} }").is_err());
    }
}
