mod dev_console;
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

fn dev(path: &str) -> Result<(), String> {
    use crossterm::event::{self, Event, KeyCode, KeyEventKind};
    use std::{io::{BufRead, BufReader}, process::{Child, Command, Stdio}, sync::mpsc::{self, Sender}, thread, time::Duration};

    fn launch(exe: &Path, path: &str, tx: &Sender<String>) -> Result<Child, String> {
        let mut child = Command::new(exe).args(["run", path])
            .stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()
            .map_err(|e| format!("cannot start application: {e}"))?;
        if let Some(stdout) = child.stdout.take() {
            let tx = tx.clone();
            thread::spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    match line {
                        Ok(line) => { if tx.send(line).is_err() { break; } }
                        Err(_) => break,
                    }
                }
            });
        }
        if let Some(stderr) = child.stderr.take() {
            let tx = tx.clone();
            thread::spawn(move || {
                for line in BufReader::new(stderr).lines() {
                    match line {
                        Ok(line) => { if tx.send(line).is_err() { break; } }
                        Err(_) => break,
                    }
                }
            });
        }
        Ok(child)
    }
    fn valid(exe: &Path, path: &str) -> bool {
        Command::new(exe).args(["check", path]).output()
            .is_ok_and(|output| output.status.success())
    }
    let exe = env::current_exe().map_err(|e| e.to_string())?;
    let mut source = fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    if !valid(&exe, path) { return Err("Fix source errors before starting dev mode".into()); }
    let (tx, rx) = mpsc::channel::<String>();
    let mut child = launch(&exe, path, &tx)?;
    let mut console = dev_console::DevConsole::new(path);
    console.status = "RUNNING".into();
    console.log("INFO", "Application started. Watching for changes.");
    let mut exit_reported = false;
    let mut quit = false;
    while !quit {
        if console.active && event::poll(Duration::from_millis(100)).unwrap_or(false) {
            if let Ok(Event::Key(key)) = event::read() {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q' | 'Q') => quit = true,
                        KeyCode::Char('r' | 'R') => {
                            if valid(&exe, path) {
                                let _ = child.kill(); let _ = child.wait();
                                child = launch(&exe, path, &tx)?;
                                console.clear_endpoint();
                                console.status = "RUNNING".into();
                                console.log("SUCCESS", "Application restarted");
                                exit_reported = false;
                            } else { console.log("ERROR", "Validation failed; keeping current application"); }
                        }
                        KeyCode::Char('t' | 'T') => {
                            console.log("INFO", "Running tests...");
                            let result = Command::new("cargo").args(["test", "--all-targets"]).output();
                            match result {
                                Ok(output) => {
                                    for line in String::from_utf8_lossy(&output.stdout).lines().chain(String::from_utf8_lossy(&output.stderr).lines()) {
                                        console.log("INFO", line);
                                    }
                                    console.log(if output.status.success() { "SUCCESS" } else { "ERROR" },
                                        if output.status.success() { "Tests passed" } else { "Tests failed" });
                                }
                                Err(error) => console.log("ERROR", &format!("Cannot run tests: {error}")),
                            }
                        }
                        KeyCode::Char('s' | 'S') => console.log("INFO", &format!("Status: {} | source: {path}", console.status)),
                        KeyCode::Char('h' | 'H') => console.log("INFO", "R Restart | T Tests | S Status | H Help | Q Quit | Up/Down Scroll"),
                        KeyCode::Up => console.scroll_up(),
                        KeyCode::Down => console.scroll_down(),
                        _ => {}
                    }
                }
            }
        } else { thread::sleep(Duration::from_millis(100)); }
        while let Ok(line) = rx.try_recv() {
            let level = if line.contains("ERROR") || line.starts_with("error") { "ERROR" }
                else if line.contains("WARN") { "WARN" }
                else if line.contains("DEBUG") { "DEBUG" }
                else { "INFO" };
            console.log(level, &line);
        }
        match fs::read(path) {
            Ok(next) if next != source => {
                source = next;
                if valid(&exe, path) {
                    let _ = child.kill(); let _ = child.wait();
                    child = launch(&exe, path, &tx)?;
                    console.clear_endpoint();
                    console.status = "RUNNING".into();
                    console.log("SUCCESS", "Source changed; application restarted");
                    exit_reported = false;
                } else { console.log("ERROR", "Source validation failed; previous application retained"); }
            }
            _ => {}
        }
        if !exit_reported {
            if let Ok(Some(status)) = child.try_wait() {
                console.status = "STOPPED".into();
                console.clear_endpoint();
                console.log("WARN", &format!("Application exited: {status}"));
                exit_reported = true;
            }
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    Ok(())
}

fn project_check(path: &str) {
    let layout = rux::project::discover(Path::new(path)).unwrap_or_else(|error| {
        eprintln!("RUX project error: {error}");
        process::exit(1);
    });
    println!("RUX project layout OK: {}", layout.root.display());
    println!("Global config: {}", layout.app.display());
    println!("Entry point: {}", layout.main.display());
    println!("Modules found: {}", layout.modules.len());
    for module in layout.modules {
        println!("  {}", module.display());
    }
}

fn help() {
    println!("RUX {}", env!("CARGO_PKG_VERSION"));
    println!("Usage: rux <command> [file.rux]");
    println!("Commands: dev, run, start, check, new, project-check, crud, crud-from");
    println!("  project-check [dir]  Validate project layout and list modules");
    println!("  dev [file.rux]    Watch and restart on source changes");
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
        [_, command] if command == "project-check" => {
            project_check(".");
        }
        [_, command, path] if command == "project-check" => {
            project_check(path);
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
        [_, command] if command == "dev" => {
            if let Err(error) = dev("main.rux") { eprintln!("RUX dev error: {error}"); process::exit(1); }
        }
        [_, command, path] if command == "dev" => {
            if let Err(error) = dev(path) { eprintln!("RUX dev error: {error}"); process::exit(1); }
        }
        [_, command] if command == "check" || command == "run" || command == "start" => {
            execute_file(command, "main.rux");
        }
        [_, command, path] if command == "check" || command == "run" || command == "start" => {
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
