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
    use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
    use std::{process::{Child, Command}, thread, time::Duration};

    struct TerminalGuard(bool);
    impl Drop for TerminalGuard {
        fn drop(&mut self) {
            if self.0 { let _ = disable_raw_mode(); }
        }
    }
    fn spawn(executable: &Path, path: &str) -> Result<Child, String> {
        Command::new(executable).args(["run", path]).spawn()
            .map_err(|e| format!("cannot start RUX application: {e}"))
    }
    fn check(executable: &Path, path: &str) -> bool {
        Command::new(executable).args(["check", path]).status()
            .is_ok_and(|status| status.success())
    }
    fn restart(child: &mut Child, executable: &Path, path: &str) -> Result<(), String> {
        if !check(executable, path) {
            eprintln!("RUX dev: validation failed; keeping previous application running");
            return Ok(());
        }
        let _ = child.kill();
        let _ = child.wait();
        *child = spawn(executable, path)?;
        println!("RUX dev: application restarted");
        Ok(())
    }

    let executable = env::current_exe().map_err(|e| format!("cannot locate RUX executable: {e}"))?;
    let mut last_source = fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    if !check(&executable, path) {
        return Err("Fix the RUX source errors before starting dev mode".into());
    }
    let mut child = spawn(&executable, path)?;
    let interactive = enable_raw_mode().is_ok();
    let _terminal = TerminalGuard(interactive);
    println!("\\r\\n  RUX {}  |  DEVELOPMENT", env!("CARGO_PKG_VERSION"));
    println!("  Source: {path}");
    println!("  [R] Restart  [T] Tests  [S] Status  [H] Help  [Q] Quit");
    println!("  Watching for changes...");

    let mut exit_reported = false;
    loop {
        if interactive && event::poll(Duration::from_millis(100)).unwrap_or(false) {
            if let Ok(Event::Key(key)) = event::read() {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q' | 'Q') => break,
                        KeyCode::Char('r' | 'R') => {
                            restart(&mut child, &executable, path)?;
                            exit_reported = false;
                        }
                        KeyCode::Char('t' | 'T') => {
                            println!("\\r\\nRUX dev: running tests...");
                            let status = Command::new("cargo").args(["test", "--all-targets"]).status();
                            println!("RUX dev: tests {}", if matches!(status, Ok(s) if s.success()) { "PASSED" } else { "FAILED" });
                        }
                        KeyCode::Char('s' | 'S') => {
                            let status = child.try_wait().ok().flatten();
                            println!("\\r\\nRUX dev: {} | source: {path}", if status.is_some() { "STOPPED" } else { "RUNNING" });
                        }
                        KeyCode::Char('h' | 'H') => println!("\\r\\n[R] Restart  [T] Run tests  [S] Show status  [H] Help  [Q] Quit"),
                        _ => {}
                    }
                }
            }
        } else {
            thread::sleep(Duration::from_millis(100));
        }
        match fs::read(path) {
            Ok(source) if source != last_source => {
                last_source = source;
                thread::sleep(Duration::from_millis(200));
                println!("\\r\\nRUX dev: source changed");
                restart(&mut child, &executable, path)?;
                exit_reported = false;
            }
            Ok(_) => {}
            Err(error) => eprintln!("RUX dev: cannot read {path}: {error}"),
        }
        if !exit_reported {
            if let Ok(Some(status)) = child.try_wait() {
                eprintln!("RUX dev: application exited ({status}); waiting for changes or [R]");
                exit_reported = true;
            }
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    println!("\\r\\nRUX dev: stopped");
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
