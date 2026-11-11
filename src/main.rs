use std::{env, fs, path::Path, process};

fn create_project(name: &str) -> Result<(), String> {
    if name.is_empty() || !name.chars().next().unwrap().is_ascii_alphabetic()
        || !name.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        return Err("project name must start with a letter and contain only letters, numbers, '-' or '_'".into());
    }
    let directory = Path::new(name);
    if directory.exists() {
        return Err(format!("directory already exists: {name}"));
    }
    fs::create_dir(directory).map_err(|e| format!("cannot create project: {e}"))?;
    let app_name = name.replace('-', "_");
    let source = format!("app {app_name} {{\n    fn main() {{\n        print(\"Hello, RUX!\")\n    }}\n}}\n");
    fs::write(directory.join("main.rux"), source).map_err(|e| format!("cannot create main.rux: {e}"))?;
    println!("Created RUX project: {name}");
    println!("  cd {name}");
    println!("  rux run");
    Ok(())
}

fn execute_file(command: &str, path: &str) {
    let source = fs::read_to_string(path).unwrap_or_else(|error| {
        eprintln!("cannot read {path}: {error}");
        process::exit(1);
    });
    if source.lines().any(|line| line.trim_start().starts_with("data ")) {
        let (name, _) = rux::crud_schema::parse_typed_schema(&source).unwrap_or_else(|error| {
            eprintln!("RUX schema error: {error}");
            process::exit(1);
        });
        if command == "check" {
            println!("Schema OK: {path}");
            return;
        }
        let (_, fields) = rux::crud_schema::parse_typed_schema(&source).unwrap_or_else(|error| {
            eprintln!("RUX schema error: {error}");
            process::exit(1);
        });
        let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap_or_else(|error| {
            eprintln!("cannot initialize runtime: {error}");
            process::exit(1);
        });
        if let Err(error) = runtime.block_on(rux::native_crud::serve(&name, fields)) {
            eprintln!("RUX native CRUD error: {error}");
            process::exit(1);
        }
        return;
    }
    if command == "check" {
        match rux::check(&source) {
            Ok(()) => println!("Syntax OK: {path}"),
            Err(error) => {
                eprintln!("Syntax error: {error}");
                process::exit(1);
            }
        }
    } else {
        match rux::run(&source) {
            Ok(output) => print!("{output}"),
            Err(error) => {
                eprintln!("RUX error: {error}");
                process::exit(1);
            }
        }
    }
}

fn help() {
    println!("RUX {}", env!("CARGO_PKG_VERSION"));
    println!("Usage: rux <command> [file.rux]");
    println!("Commands: dev, run, start, check, new, crud, crud-from");
    println!("  dev [file.rux]    Development execution (no hot reload yet)");
    println!("  start [file.rux]  Start application");
    println!("  --version        Print version");
    println!("  --help           Show this help");
}

fn main() {
    let args: Vec<String> = env::args().collect();
    match args.as_slice() {
        [_, flag] if flag == "--help" || flag == "-h" || flag == "help" => help(),
        [_, flag] if flag == "--version" || flag == "-V" => {
            println!("rux {}", env!("CARGO_PKG_VERSION"));
        }
        [_, command, path] if command == "crud-from" => {
            if let Err(error) = rux::crud_schema::generate(path) {
                eprintln!("RUX error: {error}");
                process::exit(1);
            }
        }
        [_, command, name, fields @ ..] if command == "crud" => {
            if let Err(error) = rux::crud::create(name, fields) {
                eprintln!("RUX error: {error}");
                process::exit(1);
            }
        }
        [_, command, name] if command == "new" => {
            if let Err(error) = create_project(name) {
                eprintln!("RUX error: {error}");
                process::exit(1);
            }
        }
        [_, command] if command == "check" || command == "run" || command == "start" || command == "dev" => {
            execute_file(command, "main.rux");
        }
        [_, command, path] if command == "check" || command == "run" => {
            execute_file(command, path);
        }
        _ => {
            help();
            process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_invalid_project_names() {
        assert!(create_project("../bad").is_err());
        assert!(create_project("1bad").is_err());
        assert!(create_project("").is_err());
    }
}
