use std::{env, fs, process};

fn main() {
    let args: Vec<String> = env::args().collect();
    match args.as_slice() {
        [_, flag] if flag == "--version" || flag == "-V" => {
            println!("rux {}", env!("CARGO_PKG_VERSION"));
        }
        [_, command, path] if command == "check" => {
            let source = fs::read_to_string(path).unwrap_or_else(|error| {
                eprintln!("cannot read {path}: {error}");
                process::exit(1);
            });
            match rux::check(&source) {
                Ok(()) => println!("Syntax OK: {path}"),
                Err(error) => { eprintln!("Syntax error: {error}"); process::exit(1); }
            }
        }
        _ => {
            eprintln!("Usage: rux --version | rux check <file.rux>");
            process::exit(2);
        }
    }
}
