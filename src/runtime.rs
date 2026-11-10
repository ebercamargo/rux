use std::collections::HashMap;
use crate::parser::{Expr, Program, Statement};
use crate::lexer::Token;

#[derive(Clone)]
struct Binding { value: String, mutable: bool }

pub fn run(program: &Program) -> Result<String, String> {
    let main = program.functions.iter().find(|f| f.name == "main").ok_or("missing fn main()")?;
    let mut variables = HashMap::new();
    let mut output = String::new();
    let mut budget = 100_000usize;
    execute(&main.body, &mut variables, &mut output, &mut budget)?;
    Ok(output)
}

fn execute(body: &[Statement], variables: &mut HashMap<String, Binding>, output: &mut String, budget: &mut usize) -> Result<(), String> {
    for statement in body {
        if *budget == 0 { return Err("execution limit exceeded (100000 statements)".into()); }
        *budget -= 1;
        match statement {
            Statement::Bind { name, mutable, value } => {
                if variables.contains_key(name) { return Err(format!("duplicate variable: {name}")); }
                let evaluated = evaluate(value, variables)?;
                variables.insert(name.clone(), Binding { value: evaluated, mutable: *mutable });
            }
            Statement::Assign { name, value } => {
                let evaluated = evaluate(value, variables)?;
                let binding = variables.get_mut(name).ok_or_else(|| format!("undefined variable: {name}"))?;
                if !binding.mutable { return Err(format!("cannot assign to immutable variable: {name}")); }
                binding.value = evaluated;
            }
            Statement::Call { name, args } if name == "print" => {
                let values: Result<Vec<_>, _> = args.iter().map(|a| evaluate(a, variables)).collect();
                output.push_str(&values?.join(" "));
                output.push('\n');
            }
            Statement::Call { name, .. } => return Err(format!("unknown function: {name}")),
            Statement::When { condition, then_body, else_body } => {
                let value = evaluate(condition, variables)?;
                match value.as_str() {
                    "true" => execute(then_body, variables, output, budget)?,
                    "false" => execute(else_body, variables, output, budget)?,
                    _ => return Err("when condition must be Bool".into()),
                }
            }
            Statement::LoopWhile { condition, body } => {
                loop {
                    if *budget == 0 { return Err("execution limit exceeded (100000 statements)".into()); }
                    *budget -= 1;
                    match evaluate(condition, variables)?.as_str() {
                        "true" => execute(body, variables, output, budget)?,
                        "false" => break,
                        _ => return Err("loop while condition must be Bool".into()),
                    }
                }
            }
            Statement::Loop { count, body } => {
                let value = evaluate(count, variables)?;
                let times: usize = value.parse().map_err(|_| "loop count must be a nonnegative integer")?;
                if times > 100_000 { return Err("loop count exceeds safety limit (100000)".into()); }
                for _ in 0..times { execute(body, variables, output, budget)?; }
            }
        }
    }
    Ok(())
}

fn evaluate(expr: &Expr, variables: &HashMap<String, Binding>) -> Result<String, String> {
    match expr {
        Expr::Text(value) | Expr::Number(value) => Ok(value.clone()),
        Expr::Bool(value) => Ok(value.to_string()),
        Expr::Binary { left, op, right } => {
            let a = evaluate(left, variables)?;
            let b = evaluate(right, variables)?;
            if matches!(op, Token::EqualEqual | Token::BangEqual) {
                return Ok((if matches!(op, Token::EqualEqual) { a == b } else { a != b }).to_string());
            }
            if matches!(op, Token::Plus) && (!a.parse::<i64>().is_ok() || !b.parse::<i64>().is_ok()) {
                return Ok(format!("{a}{b}"));
            }
            let x: i64 = a.parse().map_err(|_| "arithmetic requires integer operands")?;
            let y: i64 = b.parse().map_err(|_| "arithmetic requires integer operands")?;
            let numeric = match op {
                Token::Plus => x.checked_add(y),
                Token::Minus => x.checked_sub(y),
                Token::Star => x.checked_mul(y),
                Token::Slash => x.checked_div(y),
                Token::Less => return Ok((x < y).to_string()),
                Token::Greater => return Ok((x > y).to_string()),
                Token::LessEqual => return Ok((x <= y).to_string()),
                Token::GreaterEqual => return Ok((x >= y).to_string()),
                _ => return Err("unsupported operator".into()),
            };
            numeric.map(|n| n.to_string()).ok_or_else(|| "integer overflow or division by zero".into())
        },
        Expr::Variable(name) => variables.get(name).map(|b| b.value.clone()).ok_or_else(|| format!("undefined variable: {name}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{lexer, parser};
    fn execute_source(source: &str) -> Result<String, String> {
        run(&parser::parse(&lexer::lex(source)?)?)
    }
    #[test]
    fn hello_world_runs() {
        assert_eq!(execute_source("app Hello { fn main() { let message = \"Hello, World!\"; print(message) } }").unwrap(), "Hello, World!\n");
    }
    #[test]
    fn mutable_reassignment() {
        assert_eq!(execute_source("app A { fn main() { var name = \"A\"; name = \"B\"; print(name) } }").unwrap(), "B\n");
    }
    #[test]
    fn immutable_reassignment_fails() {
        assert!(execute_source("app A { fn main() { let name = \"A\"; name = \"B\" } }").unwrap_err().contains("immutable"));
    }
    #[test]
    fn when_else() {
        assert_eq!(execute_source("app A { fn main() { when false { print(\"bad\") } else { print(\"ok\") } } }").unwrap(), "ok\n");
    }
    #[test]
    fn counted_loop() {
        assert_eq!(execute_source("app A { fn main() { loop 3 { print(\"Hi\") } } }").unwrap(), "Hi\nHi\nHi\n");
    }
    #[test]
    fn arithmetic_and_comparison() {
        assert_eq!(execute_source("app A { fn main() { var x = 2 + 3 * 4; when x == 14 { print(x / 2) } } }").unwrap(), "7\n");
    }
    #[test]
    fn loop_while_updates_counter() {
        assert_eq!(execute_source("app A { fn main() { var i = 0; loop while i < 3 { i = i + 1; print(i) } } }").unwrap(), "1\n2\n3\n");
    }
    #[test]
    fn division_by_zero_is_error() {
        assert!(execute_source("app A { fn main() { print(5 / 0) } }").is_err());
    }
    #[test]
    fn rejects_undefined_variable() {
        assert!(execute_source("app A { fn main() { print(missing) } }").is_err());
    }
    #[test]
    fn rejects_unknown_function() {
        assert!(execute_source("app A { fn main() { unknown() } }").is_err());
    }
    #[test]
    fn requires_main() {
        assert!(execute_source("app A { fn other() {} }").is_err());
    }
}
