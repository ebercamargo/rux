//! Project layout discovery and strict, non-secret configuration metadata.
//! This is groundwork for the future multi-file compiler, not a module linker.
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectLayout {
    pub root: PathBuf,
    pub app: PathBuf,
    pub main: PathBuf,
    pub modules: Vec<PathBuf>,
}

pub fn discover(root: &Path) -> Result<ProjectLayout, String> {
    let root = root.canonicalize().map_err(|e| format!("cannot open project root: {e}"))?;
    let app = root.join("app.rux");
    let main = root.join("main.rux");
    if !app.is_file() { return Err("project requires app.rux".into()); }
    if !main.is_file() { return Err("project requires main.rux".into()); }
    let mut modules = Vec::new();
    let folder = root.join("modules");
    if folder.is_dir() { collect_modules(&folder, &mut modules, 0)?; }
    modules.sort();
    Ok(ProjectLayout { root, app, main, modules })
}

fn collect_modules(folder: &Path, modules: &mut Vec<PathBuf>, depth: usize) -> Result<(), String> {
    if depth > 32 { return Err("module directory nesting exceeds 32 levels".into()); }
    for entry in std::fs::read_dir(folder).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_symlink() { return Err(format!("symlinks are not supported in modules: {}", entry.path().display())); }
        if kind.is_dir() { collect_modules(&entry.path(), modules, depth + 1)?; }
        else if kind.is_file() && entry.path().extension().is_some_and(|ext| ext == "rux") {
            if modules.len() >= 10_000 { return Err("project exceeds 10000 RUX modules".into()); }
            modules.push(entry.path());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn requires_global_app_and_main() {
        let root = std::env::temp_dir().join(format!("rux-project-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        assert!(discover(&root).is_err());
        std::fs::write(root.join("app.rux"), "app Demo {}").unwrap();
        std::fs::write(root.join("main.rux"), "").unwrap();
        let project = discover(&root).unwrap();
        assert!(project.modules.is_empty());
        std::fs::create_dir_all(root.join("modules/clientes")).unwrap();
        std::fs::write(root.join("modules/clientes/cliente.rux"), "data Cliente {}").unwrap();
        assert_eq!(discover(&root).unwrap().modules.len(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }
}
