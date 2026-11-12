use std::collections::HashMap;
use crate::lexer::Token;
use crate::parser::{Expr, Program, Statement};

#[derive(Clone)]
struct Binding { value: String, mutable: bool }

struct Context<'a> {
    program: &'a Program,
    output: String,
    budget: usize,
    depth: usize,
}

pub fn run(program: &Program) -> Result<String, String> {
    let mut context = Context { program, output: String::new(), budget: 100_000, depth: 0 };
    context.call("main", Vec::new())?;
    Ok(context.output)
}

impl Context<'_> {
    fn tick(&mut self) -> Result<(), String> {
        if self.budget == 0 { return Err("execution limit exceeded (100000 statements)".into()); }
        self.budget -= 1;
        Ok(())
    }

    fn call(&mut self, name: &str, args: Vec<String>) -> Result<String, String> {
        let function = self.program.functions.iter().find(|f| f.name == name)
            .ok_or_else(|| format!("unknown function: {name}"))?;
        if args.len() != function.params.len() {
            return Err(format!("function {name} expects {} arguments, got {}", function.params.len(), args.len()));
        }
        if self.depth >= 128 { return Err("function call depth exceeded (128)".into()); }
        let mut variables = HashMap::new();
        for (param, value) in function.params.iter().zip(args) {
            if variables.insert(param.clone(), Binding { value, mutable: false }).is_some() {
                return Err(format!("duplicate parameter: {param}"));
            }
        }
        self.depth += 1;
        let result = self.execute(&function.body, &mut variables);
        self.depth -= 1;
        Ok(result?.unwrap_or_default())
    }

    fn execute(&mut self, body: &[Statement], variables: &mut HashMap<String, Binding>) -> Result<Option<String>, String> {
        for statement in body {
            self.tick()?;
            match statement {
                Statement::Bind { name, mutable, value } => {
                    if variables.contains_key(name) { return Err(format!("duplicate variable: {name}")); }
                    let evaluated = self.evaluate(value, variables)?;
                    variables.insert(name.clone(), Binding { value: evaluated, mutable: *mutable });
                }
                Statement::Assign { name, value } => {
                    let evaluated = self.evaluate(value, variables)?;
                    let binding = variables.get_mut(name).ok_or_else(|| format!("undefined variable: {name}"))?;
                    if !binding.mutable { return Err(format!("cannot assign to immutable variable: {name}")); }
                    binding.value = evaluated;
                }
                Statement::Call { name, args } => {
                    let values = args.iter().map(|a| self.evaluate(a, variables)).collect::<Result<Vec<_>, _>>()?;
                    if name == "print" {
                        self.output.push_str(&values.join(" "));
                        self.output.push('\n');
                    } else { self.call(name, values)?; }
                }
                Statement::Return(expr) => return Ok(Some(self.evaluate(expr, variables)?)),
                Statement::When { condition, then_body, else_body } => {
                    let value = self.evaluate(condition, variables)?;
                    let branch = match value.as_str() {
                        "true" => then_body,
                        "false" => else_body,
                        _ => return Err("when condition must be Bool".into()),
                    };
                    if let Some(result) = self.execute(branch, variables)? { return Ok(Some(result)); }
                }
                Statement::LoopWhile { condition, body } => {
                    loop {
                        self.tick()?;
                        match self.evaluate(condition, variables)?.as_str() {
                            "true" => if let Some(result) = self.execute(body, variables)? { return Ok(Some(result)); },
                            "false" => break,
                            _ => return Err("loop while condition must be Bool".into()),
                        }
                    }
                }
                Statement::Loop { count, body } => {
                    let value = self.evaluate(count, variables)?;
                    let times: usize = value.parse().map_err(|_| "loop count must be a nonnegative integer")?;
                    if times > 100_000 { return Err("loop count exceeds safety limit (100000)".into()); }
                    for _ in 0..times {
                        if let Some(result) = self.execute(body, variables)? { return Ok(Some(result)); }
                    }
                }
            }
        }
        Ok(None)
    }

    fn evaluate(&mut self, expr: &Expr, variables: &HashMap<String, Binding>) -> Result<String, String> {
        match expr {
            Expr::Text(value) | Expr::Number(value) => Ok(value.clone()),
            Expr::Bool(value) => Ok(value.to_string()),
            Expr::Variable(name) => variables.get(name).map(|b| b.value.clone()).ok_or_else(|| format!("undefined variable: {name}")),
            Expr::Call { name, args } => {
                let values = args.iter().map(|a| self.evaluate(a, variables)).collect::<Result<Vec<_>, _>>()?;
                if name == "print" { return Err("print cannot be used as an expression".into()); }
                self.call(name, values)
            }
            Expr::Binary { left, op, right } => {
                let a = self.evaluate(left, variables)?;
                let b = self.evaluate(right, variables)?;
                if matches!(op, Token::EqualEqual | Token::BangEqual) {
                    return Ok((if matches!(op, Token::EqualEqual) { a == b } else { a != b }).to_string());
                }
                if matches!(op, Token::Plus) && (a.parse::<i64>().is_err() || b.parse::<i64>().is_err()) {
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
            }
        }
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
    fn negative_arithmetic_runs() {
        assert_eq!(execute_source("app A { fn main() { print(-5 + 2) } }").unwrap(), "-3\n");
    }
    #[test]
    fn unicode_variables_run() {
        assert_eq!(execute_source("app Gestão { fn main() { let descrição = \"Olá\"; print(descrição) } }").unwrap(), "Olá\n");
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
    fn functions_with_arguments_and_return() {
        let source = "app A { fn double(value) { return value * 2 } fn main() { print(double(6)) } }";
        assert_eq!(execute_source(source).unwrap(), "12\n");
    }
    #[test]
    fn return_propagates_from_nested_block() {
        let source = "app A { fn choose(value) { when true { return value } return 0 } fn main() { print(choose(7)) } }";
        assert_eq!(execute_source(source).unwrap(), "7\n");
    }
    #[test]
    fn wrong_arity_fails() {
        assert!(execute_source("app A { fn f(x) { return x } fn main() { print(f()) } }").is_err());
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
