//! Optional declarative security blocks for CRUD-style RUX applications.
//! Role names are never privileged; runtime permissions are explicit.
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleDeclaration {
    pub name: String,
    pub permissions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteRequirement {
    pub method: String,
    pub path: String,
    pub permission: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SecuritySpec {
    pub roles: Vec<RoleDeclaration>,
    pub routes: Vec<RouteRequirement>,
}

impl SecuritySpec {
    pub fn route_permissions(&self) -> HashMap<(String, String), String> {
        self.routes.iter().map(|rule| (
            (rule.method.clone(), rule.path.clone()), rule.permission.clone()
        )).collect()
    }
}

enum Block {
    Security,
    Role { name: String, grants: Vec<String> },
    Route { method: String, path: String, permission: Option<String> },
}

fn supported_crud_route(method: &str, path: &str) -> bool {
    matches!((method, path),
        ("GET", "/api/records") |
        ("POST", "/api/records") |
        ("PUT", "/api/records/{id}") |
        ("DELETE", "/api/records/{id}"))
}

/// Extract an optional security block from an app's top-level scope.
/// Removed lines are replaced by blank lines, preserving original diagnostics.
pub fn parse_source(source: &str) -> Result<(String, Option<SecuritySpec>), String> {
    let mut result = String::new();
    let mut depth = 0usize;
    let mut block: Option<Block> = None;
    let mut spec: Option<SecuritySpec> = None;
    let mut seen_roles = HashSet::new();
    let mut seen_routes = HashSet::new();

    for (line_no, raw) in source.lines().enumerate() {
        let line = raw.split("//").next().unwrap_or("").trim();
        if let Some(current) = block.take() {
            result.push('\n');
            let location = line_no + 1;
            block = Some(match current {
                Block::Security => {
                    if line.is_empty() { Block::Security }
                    else if line == "}" { continue; }
                    else if let Some(name) = line.strip_prefix("role ").and_then(|s| s.strip_suffix(" {")) {
                        if !super::policy::valid_role_name(name) {
                            return Err(format!("line {location}: invalid security role name: {name}"));
                        }
                        if !seen_roles.insert(name.to_owned()) {
                            return Err(format!("line {location}: duplicate security role: {name}"));
                        }
                        Block::Role { name: name.into(), grants: Vec::new() }
                    } else if let Some(declaration) = line.strip_prefix("route ").and_then(|s| s.strip_suffix(" {")) {
                        let (method, quoted_path) = declaration.split_once(' ')
                            .ok_or_else(|| format!("line {location}: expected route METHOD \"/path\" {{"))?;
                        let path = quoted_path.strip_prefix('"').and_then(|s| s.strip_suffix('"'))
                            .ok_or_else(|| format!("line {location}: route path must be quoted"))?;
                        if !supported_crud_route(method, path) {
                            return Err(format!("line {location}: route {method} {path} is not a supported CRUD endpoint yet; custom route handlers are not implemented"));
                        }
                        if !seen_routes.insert((method.to_owned(), path.to_owned())) {
                            return Err(format!("line {location}: duplicate route policy {method} {path}"));
                        }
                        Block::Route { method: method.into(), path: path.into(), permission: None }
                    } else {
                        return Err(format!("line {location}: expected role Name {{, route METHOD \"/path\" {{, or }}"));
                    }
                }
                Block::Role { name, mut grants } => {
                    if line.is_empty() { Block::Role { name, grants } }
                    else if line == "}" {
                        spec.as_mut().expect("security spec").roles.push(RoleDeclaration { name, permissions: grants });
                        Block::Security
                    } else if let Some(permission) = line.strip_prefix("allow ") {
                        if !super::policy::valid_permission(permission) {
                            return Err(format!("line {location}: invalid permission: {permission}"));
                        }
                        if !grants.iter().any(|p| p == permission) {
                            grants.push(permission.to_owned());
                        }
                        Block::Role { name, grants }
                    } else {
                        return Err(format!("line {location}: expected allow resource.action or }}"));
                    }
                }
                Block::Route { method, path, permission } => {
                    if line.is_empty() { Block::Route { method, path, permission } }
                    else if line == "}" {
                        let permission = permission
                            .ok_or_else(|| format!("line {location}: route policy must require a permission"))?;
                        spec.as_mut().expect("security spec").routes.push(RouteRequirement { method, path, permission });
                        Block::Security
                    } else if let Some(grant) = line.strip_prefix("require ") {
                        if permission.is_some() {
                            return Err(format!("line {location}: route policy has multiple require clauses"));
                        }
                        if !super::policy::valid_permission(grant) {
                            return Err(format!("line {location}: invalid required permission: {grant}"));
                        }
                        Block::Route { method, path, permission: Some(grant.to_owned()) }
                    } else {
                        return Err(format!("line {location}: expected require resource.action or }}"));
                    }
                }
            });
            continue;
        }
        if line == "security {" {
            if depth != 1 {
                return Err(format!("line {}: security must be inside the app, outside data declarations", line_no + 1));
            }
            if spec.is_some() {
                return Err(format!("line {}: duplicate security block", line_no + 1));
            }
            spec = Some(SecuritySpec::default());
            block = Some(Block::Security);
            result.push('\n');
            continue;
        }
        result.push_str(raw);
        result.push('\n');
        let opens = line.bytes().filter(|b| *b == b'{').count();
        let closes = line.bytes().filter(|b| *b == b'}').count();
        if closes > depth + opens {
            return Err(format!("line {}: unexpected closing brace", line_no + 1));
        }
        depth = depth + opens - closes;
    }
    if block.is_some() {
        return Err(format!("line {}: unclosed security declaration", source.lines().count()));
    }
    Ok((result, spec))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_roles_and_route_requirements_without_changing_line_numbers() {
        let src = "app Example {\n data Customer {\n  name: Text\n }\n security {\n  role physician {\n   allow records.read\n   allow patients.read\n  }\n  route GET \"/api/records\" {\n   require patients.read\n  }\n }\n}";
        let (clean, spec) = parse_source(src).unwrap();
        let spec = spec.unwrap();
        assert_eq!(spec.roles[0].name, "physician");
        assert_eq!(spec.roles[0].permissions, vec!["records.read", "patients.read"]);
        assert_eq!(spec.routes[0].permission, "patients.read");
        assert_eq!(clean.lines().count(), src.lines().count());
        assert_eq!(crate::crud_schema::parse_typed_schema(&clean).unwrap().0, "Example");
    }

    #[test]
    fn rejects_duplicate_roles_unsafe_paths_and_missing_permission() {
        let code = |security: &str| format!("app Test {{\n data Item {{\n  name: Text\n }}\n security {{\n {security}\n }}\n}}");
        assert!(parse_source(&code("role doctor {\n }\n role doctor {\n }")).is_err());
        assert!(parse_source(&code("route GET \"/secrets\" {\n require secrets.read\n }")).is_err());
        assert!(parse_source(&code("route GET \"/api/records\" {\n }")).is_err());
        assert!(parse_source(&code("role doctor {\n allow records.*\n }")).is_err());
        assert!(parse_source(&code("route GET \"/api/records\" {\n require records.read\n }\n route GET \"/api/records\" {\n require records.read\n }")).is_err());
        assert!(parse_source(&code("role doctor {\n allow records.read\n }")).is_ok());
    }

    #[test]
    fn empty_or_missing_security_keeps_original_crud_behavior() {
        let src = "app Test {\n data Item {\n  name: Text\n }\n}";
        let (_, policy) = parse_source(src).unwrap();
        assert!(policy.is_none());
        let src = "app Test {\n data Item {\n  name: Text\n }\n security {\n }\n}";
        assert_eq!(parse_source(src).unwrap().1.unwrap(), SecuritySpec::default());
    }
}
